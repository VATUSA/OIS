//! TMU persistence — Traffic Management Initiatives (TMIs).

use crate::auth::principal::Attribution;
use crate::repos::flow::{Expect, ReleaseHolder};
use sqlx::{PgPool, Postgres, Transaction};

use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::{
    errors::ApiError,
    models::{
        AdvisoryBody, CreateAdvisoryRequest, CreateGroundStopRequest, CreateTmiRequest, GateRule,
        GroundStopBody, IssuedCfrBody, ProgramBody, TmiBody, UpdateAdvisoryRequest,
        UpdateTmiRequest, UpsertProgramRequest,
    },
};

const SELECT: &str = "select t.id, t.requesting, t.providing, t.restriction, \
    t.start_time, t.stop_time, t.status, t.published_at, t.created_at, \
    u.display_name as author, t.structured, t.decoded \
    from tmu.tmis t left join identity.users u on u.id = t.created_by";

/// Optional filters for the TMI list. Every field `None` → every TMI.
#[derive(Debug, Default)]
pub struct TmiFilters {
    pub status: Option<String>,
    /// Structured NTML restriction kind (`MIT`, `MINIT`, `STOP`, …). Raw-typed TMIs (no structured
    /// form) never match a kind filter.
    pub kind: Option<String>,
    /// Matched against the requesting **or** providing facility, case-insensitively.
    pub facility: Option<String>,
    /// Active-during range: a TMI matches when its validity window overlaps `[from, to]`.
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

/// Shared WHERE + ordering for `list_tmis`. Bind order: `$1` status, `$2` kind, `$3` facility,
/// `$4` range end (`to`), `$5` range start (`from`).
const TMI_WHERE: &str = " where t.dismissed_at is null \
       and ($1::text is null or t.status = $1) \
       and ($2::text is null or upper(t.structured->>'kind') = upper($2)) \
       and ($3::text is null or upper(t.requesting) = upper($3) or upper(t.providing) = upper($3)) \
       and ($4::timestamptz is null or t.start_time <= $4) \
       and ($5::timestamptz is null or t.stop_time is null or t.stop_time >= $5) \
     order by t.created_at desc";

pub async fn list_tmis(pool: &PgPool, f: &TmiFilters) -> Result<Vec<TmiBody>, ApiError> {
    sqlx::query_as::<_, TmiBody>(&format!("{SELECT}{TMI_WHERE}"))
        .bind(f.status.as_deref())
        .bind(f.kind.as_deref())
        .bind(f.facility.as_deref())
        .bind(f.to)
        .bind(f.from)
        .fetch_all(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

pub async fn get_tmi(pool: &PgPool, id: &str) -> Result<Option<TmiBody>, ApiError> {
    sqlx::query_as::<_, TmiBody>(&format!("{SELECT} where t.id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// TMIs that were live at instant `at` (for historical replay): published by then, not past their
/// window, and not cancelled before then. Never-published drafts are excluded.
pub async fn list_tmis_at(pool: &PgPool, at: DateTime<Utc>) -> Result<Vec<TmiBody>, ApiError> {
    sqlx::query_as::<_, TmiBody>(&format!(
        "{SELECT} where t.published_at is not null and t.published_at <= $1 \
           and (t.stop_time is null or t.stop_time > $1) \
           and (t.ended_at is null or t.ended_at > $1) \
         order by t.created_at desc"
    ))
    .bind(at)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn create_tmi<'e, E>(
    executor: E,
    req: &CreateTmiRequest,
    created_by: &str,
) -> Result<String, ApiError>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    // A structured TMI stores its parsed fields + the decoded English; a raw one leaves both null.
    let decoded = req.structured.as_ref().map(crate::tmi::render_english);
    sqlx::query_scalar::<_, String>(
        "insert into tmu.tmis \
         (requesting, providing, restriction, structured, decoded, start_time, stop_time, created_by) \
         values ($1, $2, $3, $4, $5, coalesce($6, now()), $7, $8) returning id",
    )
    .bind(&req.requesting)
    .bind(&req.providing)
    .bind(&req.restriction)
    .bind(req.structured.as_ref().map(sqlx::types::Json))
    .bind(decoded)
    .bind(req.start_time)
    .bind(req.stop_time)
    .bind(created_by)
    .fetch_one(executor)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Update the given fields (COALESCE — omitted fields are left unchanged) **in the caller's
/// transaction**, so that a published TMI's corrected Discord row can be enqueued atomically with
/// the edit. Returns the updated row, or `None` if the TMI doesn't exist.
///
/// Shaped like [`publish_tmi`], including re-selecting rather than using `returning`: `SELECT` joins
/// `identity.users` for `author`, which `returning` cannot produce.
///
/// Returns the row **and whether any field that appears in the posted NTML line actually changed** — see
/// [`TmiEdit`]. `rows_affected` cannot answer that: a COALESCE update setting every column to its current
/// value still affects the row, so it only distinguishes "no such id" (#453 review).
///
/// `structured`/`decoded` are the exception to "omitted means unchanged" (#452). Three cases:
///
/// | request | stored breakdown |
/// | --- | --- |
/// | `structured` present | replaced, with `decoded` re-rendered from it |
/// | `restriction` present, `structured` absent | **cleared** — it no longer describes the text |
/// | neither (e.g. only `stop_time`) | unchanged |
///
/// The third case is why this cannot be a blanket clear: editing only the valid window must not throw
/// the breakdown away. COALESCE cannot express "set to null", hence the explicit flag.
///
/// Neither column appears in the NTML line, so a breakdown-only change correctly leaves
/// `line_changed` false; a structured edit reaches here with `restriction` already re-derived by the
/// handler, which is what makes it count as a changed line.
pub async fn update_tmi(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
    req: &UpdateTmiRequest,
) -> Result<Option<TmiEdit>, ApiError> {
    let clear_breakdown = req.structured.is_none() && req.restriction.is_some();
    let decoded = req.structured.as_ref().map(crate::tmi::render_english);
    // Read the pre-edit row inside the same transaction, so "did the line change" is answered against the
    // row the edit is actually applied to rather than one that may have moved under us.
    let Some(before) = get_tmi_tx(tx, id).await? else {
        return Ok(None);
    };
    let result = sqlx::query(
        "update tmu.tmis set \
            requesting = coalesce($2, requesting), \
            providing = coalesce($3, providing), \
            restriction = coalesce($4, restriction), \
            start_time = coalesce($5, start_time), \
            stop_time = coalesce($6, stop_time), \
            structured = case when $7 then null else coalesce($8, structured) end, \
            decoded = case when $7 then null else coalesce($9, decoded) end \
         where id = $1",
    )
    .bind(id)
    .bind(&req.requesting)
    .bind(&req.providing)
    .bind(&req.restriction)
    .bind(req.start_time)
    .bind(req.stop_time)
    .bind(clear_breakdown)
    .bind(req.structured.as_ref().map(sqlx::types::Json))
    .bind(decoded)
    .execute(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)?;
    if result.rows_affected() == 0 {
        return Ok(None);
    }
    let Some(tmi) = get_tmi_tx(tx, id).await? else {
        return Ok(None);
    };
    Ok(Some(TmiEdit {
        line_changed: line_fields_differ(&before, &tmi),
        tmi,
    }))
}

/// A TMI after an edit, and whether the edit changed anything the channel shows.
pub struct TmiEdit {
    pub tmi: TmiBody,
    /// True when a field carried by the posted NTML line differs from before the edit. The handler posts a
    /// revised row only then: an edit that changed nothing must not put a second identical line into a log
    /// whose whole premise is that a later line supersedes the earlier one (#453 review).
    pub line_changed: bool,
}

/// The fields the posted NTML row is built from — `tmi_publish_job`'s payload, in other words.
///
/// `restriction`, `requesting` and `providing` are the line's text and its `REQ:PROV` token; the two times
/// are its valid window, which is why a `stop_time`-only edit **must** still post. Anything outside this
/// set (`status` transitions, `author`, `decoded`) either has its own path or does not appear in the
/// channel (#453 review).
fn line_fields_differ(before: &TmiBody, after: &TmiBody) -> bool {
    before.restriction != after.restriction
        || before.requesting != after.requesting
        || before.providing != after.providing
        || before.start_time != after.start_time
        || before.stop_time != after.stop_time
}

/// `get_tmi` against a transaction, so the before/after comparison sees the edit's own snapshot.
async fn get_tmi_tx(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
) -> Result<Option<TmiBody>, ApiError> {
    sqlx::query_as::<_, TmiBody>(&format!("{SELECT} where t.id = $1"))
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| ApiError::Internal)
}

/// Publishes a draft. Returns false if the TMI isn't currently a draft.
/// Publish a draft TMI **in the caller's transaction** (so a Discord advisory job can be enqueued
/// atomically). Returns the published row, or `None` if it wasn't a draft (or is absent).
pub async fn publish_tmi(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
    published_by: &str,
) -> Result<Option<TmiBody>, ApiError> {
    let result = sqlx::query(
        "update tmu.tmis set status = 'published', published_by = $2, published_at = now() \
         where id = $1 and status = 'draft'",
    )
    .bind(id)
    .bind(published_by)
    .execute(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)?;
    if result.rows_affected() == 0 {
        return Ok(None);
    }
    sqlx::query_as::<_, TmiBody>(&format!("{SELECT} where t.id = $1"))
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| ApiError::Internal)
}

/// Cancels a draft or published TMI. Returns false if it's already terminal. `ended_at` records the
/// early close so replay stops showing it at the cancellation time.
///
/// Generic over the executor so the TMU handler can run it inside the transaction that also
/// enqueues the cancel post (#436) — the Discord row must not exist unless the TMI really cancelled —
/// while `handlers/events.rs` keeps calling it with a plain pool.
pub async fn cancel_tmi<'e, E>(executor: E, id: &str) -> Result<bool, ApiError>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query(
        "update tmu.tmis set status = 'cancelled', ended_at = coalesce(ended_at, now()) \
         where id = $1 and status in ('draft', 'published')",
    )
    .bind(id)
    .execute(executor)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

pub async fn delete_tmi(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    delete_or_retain(pool, "tmu.tmis", id).await
}

/// Delete for the published-history entities (TMIs / ground stops / GDPs): a row that was NEVER
/// published (a draft dropped without going live) is hard-deleted and never appears in replay; a
/// row that was ever published is KEPT for the historical dashboard — cancelled (with `ended_at`
/// stamped) if still active, and always marked `dismissed_at` so the TMU lists stop showing it
/// (#304: an already-expired row used to be left untouched while the delete reported success).
/// `table` is a trusted internal literal, never user input. Returns whether a row with that id existed.
pub(crate) async fn delete_or_retain(
    pool: &PgPool,
    table: &str,
    id: &str,
) -> Result<bool, ApiError> {
    let hard = sqlx::query(&format!(
        "delete from {table} where id = $1 and published_at is null"
    ))
    .bind(id)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    if hard.rows_affected() > 0 {
        return Ok(true);
    }
    let soft = sqlx::query(&format!(
        "update {table} set \
            status = case when status in ('draft', 'published') then 'cancelled' else status end, \
            ended_at = case when status in ('draft', 'published') then coalesce(ended_at, now()) \
                            else ended_at end, \
            dismissed_at = coalesce(dismissed_at, now()) \
         where id = $1"
    ))
    .bind(id)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(soft.rows_affected() > 0)
}

// --- rate programs ---

const PROGRAM_SELECT: &str = "select p.icao, p.aar, p.trail, p.mit, p.gates, \
    p.exclude_wake, p.exclude_types, p.jets_only, p.active_until, p.updated_at, \
    u.display_name as updated_by \
    from tmu.programs p left join identity.users u on u.id = p.updated_by";

pub async fn list_programs(pool: &PgPool) -> Result<Vec<ProgramBody>, ApiError> {
    sqlx::query_as::<_, ProgramBody>(&format!("{PROGRAM_SELECT} order by p.icao"))
        .fetch_all(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

pub async fn get_program(pool: &PgPool, icao: &str) -> Result<Option<ProgramBody>, ApiError> {
    sqlx::query_as::<_, ProgramBody>(&format!("{PROGRAM_SELECT} where p.icao = $1"))
        .bind(icao)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// Creates or replaces the program for an airport (vatflow "SET PROGRAM").
pub async fn upsert_program<'e, E>(
    executor: E,
    icao: &str,
    req: &UpsertProgramRequest,
    gates: &[GateRule],
    actor: &str,
) -> Result<(), ApiError>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        "insert into tmu.programs \
         (icao, aar, trail, mit, gates, exclude_wake, exclude_types, jets_only, active_until, created_by, updated_by) \
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $10) \
         on conflict (icao) do update set \
            aar = excluded.aar, trail = excluded.trail, mit = excluded.mit, \
            gates = excluded.gates, exclude_wake = excluded.exclude_wake, \
            exclude_types = excluded.exclude_types, jets_only = excluded.jets_only, \
            active_until = excluded.active_until, updated_by = excluded.updated_by",
    )
    .bind(icao)
    .bind(req.aar)
    .bind(req.trail)
    .bind(req.mit)
    .bind(sqlx::types::Json(gates))
    .bind(&req.exclude_wake)
    .bind(&req.exclude_types)
    .bind(req.jets_only)
    .bind(req.active_until)
    .bind(actor)
    .execute(executor)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(())
}

pub async fn delete_program(pool: &PgPool, icao: &str) -> Result<bool, ApiError> {
    let result = sqlx::query("delete from tmu.programs where icao = $1")
        .bind(icao)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

// --- ground stops ---

const GS_SELECT: &str = "select g.id, g.airport, g.scope, g.until, g.status, \
    g.published_at, g.updated_at, u.display_name as updated_by \
    from tmu.ground_stops g left join identity.users u on u.id = g.updated_by";

pub async fn list_ground_stops(pool: &PgPool) -> Result<Vec<GroundStopBody>, ApiError> {
    sqlx::query_as::<_, GroundStopBody>(&format!(
        "{GS_SELECT} where g.dismissed_at is null order by g.updated_at desc"
    ))
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Ground stops that were live at instant `at` (for historical replay).
pub async fn list_ground_stops_at(
    pool: &PgPool,
    at: DateTime<Utc>,
) -> Result<Vec<GroundStopBody>, ApiError> {
    sqlx::query_as::<_, GroundStopBody>(&format!(
        "{GS_SELECT} where g.published_at is not null and g.published_at <= $1 \
           and (tmu.ground_stop_until_ts(g.created_at, g.until) is null \
                or tmu.ground_stop_until_ts(g.created_at, g.until) > $1) \
           and (g.ended_at is null or g.ended_at > $1) \
         order by g.updated_at desc"
    ))
    .bind(at)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn get_ground_stop<'e, E>(
    executor: E,
    id: &str,
) -> Result<Option<GroundStopBody>, ApiError>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query_as::<_, GroundStopBody>(&format!("{GS_SELECT} where g.id = $1"))
        .bind(id)
        .fetch_optional(executor)
        .await
        .map_err(|_| ApiError::Internal)
}

/// The absolute instant a ground stop's `until` resolves to, or `None` for "until further notice".
///
/// Delegates to `tmu.ground_stop_until_ts` (migration `0014`) rather than resolving the bare HHMM in
/// Rust. That function is what the cleanup job uses to expire a stop, and it resolves relative to
/// `created_at` — not to now — so a draft created at 1500 and published at 1700 with `until` 1630 ends
/// *tomorrow* at 1630 by the system's reckoning. Reimplementing the rule here would make the generated
/// advisory state an end the system does not enforce, which is the drift #508 exists to prevent.
pub(crate) async fn ground_stop_until_instant(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
) -> Result<Option<DateTime<Utc>>, ApiError> {
    sqlx::query_scalar::<_, Option<DateTime<Utc>>>(
        "select tmu.ground_stop_until_ts(created_at, until) from tmu.ground_stops where id = $1",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
    .map(Option::flatten)
    .map_err(|_| ApiError::Internal)
}

pub async fn create_ground_stop<'e, E>(
    executor: E,
    req: &CreateGroundStopRequest,
    scope: &str,
    until: Option<&str>,
    actor: &str,
) -> Result<String, ApiError>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query_scalar::<_, String>(
        "insert into tmu.ground_stops (airport, scope, until, created_by, updated_by) \
         values ($1, $2, $3, $4, $4) returning id",
    )
    .bind(&req.airport)
    .bind(scope)
    .bind(until)
    .bind(actor)
    .fetch_one(executor)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Publishes a draft ground stop. Returns false if it isn't currently a draft.
pub async fn publish_ground_stop<'e, E>(
    executor: E,
    id: &str,
    published_by: &str,
) -> Result<bool, ApiError>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let result = sqlx::query(
        "update tmu.ground_stops set status = 'published', published_by = $2, published_at = now() \
         where id = $1 and status = 'draft'",
    )
    .bind(id)
    .bind(published_by)
    .execute(executor)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

/// Cancels a draft or published ground stop. Returns false if it's already terminal.
pub async fn cancel_ground_stop(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "update tmu.ground_stops set status = 'cancelled', ended_at = coalesce(ended_at, now()) \
         where id = $1 and status in ('draft', 'published')",
    )
    .bind(id)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

pub async fn delete_ground_stop(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    delete_or_retain(pool, "tmu.ground_stops", id).await
}

// --- issued CFRs ---

/// Locked wheels-up times (callsign -> wheels_up) for one metered airport.
pub async fn issued_cfr_map(
    pool: &PgPool,
    airport: &str,
) -> Result<HashMap<String, DateTime<Utc>>, ApiError> {
    let rows = sqlx::query_as::<_, (String, DateTime<Utc>)>(
        "select callsign, wheels_up from tmu.issued_cfrs where airport = $1",
    )
    .bind(airport)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(rows.into_iter().collect())
}

/// Every issued CFR as (callsign, airport, wheels_up).
pub async fn all_issued_cfrs(
    pool: &PgPool,
) -> Result<Vec<(String, String, DateTime<Utc>)>, ApiError> {
    sqlx::query_as::<_, (String, String, DateTime<Utc>)>(
        "select callsign, airport, wheels_up from tmu.issued_cfrs",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Every issued CFR's version (callsign -> version), for the departures list's `cfr_version`.
pub async fn issued_cfr_versions(pool: &PgPool) -> Result<HashMap<String, i64>, ApiError> {
    let rows = sqlx::query_as::<_, (String, i64)>("select callsign, version from tmu.issued_cfrs")
        .fetch_all(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(rows.into_iter().collect())
}

/// Issue (or re-issue) a CFR, returning its new version — or `None` when `expect` did not hold and
/// nothing was written (#585). Same contract as `flow_repo::upsert_release`.
pub async fn upsert_issued_cfr(
    pool: &PgPool,
    callsign: &str,
    airport: &str,
    wheels_up: DateTime<Utc>,
    by: &Attribution,
    expect: Option<Expect>,
) -> Result<Option<i64>, ApiError> {
    let sql = match expect {
        None => {
            "insert into tmu.issued_cfrs as c (callsign, airport, wheels_up, issued_by, issued_by_actor) \
             values ($1, $2, $3, $4, $5) \
             on conflict (callsign) do update set \
                airport = excluded.airport, wheels_up = excluded.wheels_up, \
                issued_by = excluded.issued_by, issued_by_actor = excluded.issued_by_actor, \
                issued_at = now(), version = c.version + 1 \
             returning c.version"
        }
        Some(Expect::Absent) => {
            "insert into tmu.issued_cfrs (callsign, airport, wheels_up, issued_by, issued_by_actor) \
             values ($1, $2, $3, $4, $5) \
             on conflict (callsign) do nothing \
             returning version"
        }
        Some(Expect::Version(_)) => {
            "update tmu.issued_cfrs set \
                airport = $2, wheels_up = $3, issued_by = $4, issued_by_actor = $5, \
                issued_at = now(), version = version + 1 \
             where callsign = $1 and version = $6 \
               and ($7::text is null or issued_by_actor = $7) \
             returning version"
        }
    };
    let query = sqlx::query_scalar::<_, i64>(sql)
        .bind(callsign)
        .bind(airport)
        .bind(wheels_up)
        .bind(&by.user_id)
        .bind(&by.actor_id);
    // `$7` is the machine that must already hold the CFR (#585 review).
    let query = match expect {
        Some(Expect::Version(v)) => query.bind(v).bind(by.machine_actor()),
        _ => query,
    };
    query
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// Who holds a CFR now, and at what version — see `flow_repo::ReleaseHolder` (#585).
pub async fn cfr_holder(pool: &PgPool, callsign: &str) -> Result<Option<ReleaseHolder>, ApiError> {
    let row = sqlx::query_as::<_, (i64, Option<String>, Option<String>)>(
        "select c.version, \
            case when a.actor_type in ('service_account', 'api_key') then a.id end, \
            case when a.actor_type in ('service_account', 'api_key') then a.display_name end \
         from tmu.issued_cfrs c left join access.actors a on a.id = c.issued_by_actor \
         where c.callsign = $1",
    )
    .bind(callsign)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(row.map(|(version, id, name)| ReleaseHolder {
        version,
        machine: id.map(|id| (id, name.unwrap_or_default())),
    }))
}

pub async fn get_issued_cfr(
    pool: &PgPool,
    callsign: &str,
) -> Result<Option<IssuedCfrBody>, ApiError> {
    sqlx::query_as::<_, IssuedCfrBody>(
        "select c.callsign, c.airport, c.wheels_up, \
                coalesce(u.display_name, a.display_name) as issued_by, c.issued_at \
         from tmu.issued_cfrs c left join identity.users u on u.id = c.issued_by \
         left join access.actors a on a.id = c.issued_by_actor \
         where c.callsign = $1",
    )
    .bind(callsign)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Release (delete) a CFR, only at `version` when given and, for a machine, only if it already
/// holds it (#585). The holder comes from `by`, not the caller, for the reason `delete_release`
/// gives. Returns whether a row was removed.
pub async fn delete_issued_cfr(
    pool: &PgPool,
    callsign: &str,
    version: Option<i64>,
    by: &Attribution,
) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "delete from tmu.issued_cfrs where callsign = $1 and ($2::bigint is null or version = $2) \
           and ($3::text is null or issued_by_actor = $3)",
    )
    .bind(callsign)
    .bind(version)
    .bind(by.machine_actor())
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

/// Opportunistic cleanup: drop CFRs issued long ago (their flights have since departed).
pub async fn prune_stale_cfrs(pool: &PgPool) -> Result<(), ApiError> {
    sqlx::query("delete from tmu.issued_cfrs where issued_at < now() - interval '12 hours'")
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(())
}

// --- cleanup ---

/// Rows touched by a cleanup pass.
pub struct CleanupStats {
    pub expired: u64,
    pub deleted: u64,
    /// Advisories cancelled by this pass (#537) — counted separately because an advisory is neither
    /// expired nor deleted: it is cancelled, which is a document state the others do not have.
    pub advisories_cancelled: u64,
}

/// Auto-expire finished restrictions/ground stops, then delete anything that ended (or was
/// cancelled) more than an hour ago — leaving a grace window where finished items still
/// show as `expired` before they disappear.
pub async fn run_cleanup(pool: &PgPool) -> Result<CleanupStats, ApiError> {
    let internal = |_| ApiError::Internal;

    // 1. Expire actives whose end has passed (so the UI reflects it during the grace hour).
    let e_tmi = sqlx::query(
        "update tmu.tmis set status = 'expired' \
         where status in ('draft', 'published') \
           and stop_time is not null and stop_time < now()",
    )
    .execute(pool)
    .await
    .map_err(internal)?;

    let e_gs = sqlx::query(
        "update tmu.ground_stops set status = 'expired' \
         where status in ('draft', 'published') \
           and tmu.ground_stop_until_ts(created_at, until) is not null \
           and tmu.ground_stop_until_ts(created_at, until) < now()",
    )
    .execute(pool)
    .await
    .map_err(internal)?;

    // 2. Delete records whose end (or cancellation) was more than an hour ago — but ONLY those that
    // were never published. Anything that was published is kept (the historical dashboard replays
    // it); published rows are pruned later, on the stats retention window, by `prune_history`.
    let d_tmi = sqlx::query(
        "delete from tmu.tmis \
         where published_at is null \
           and ((stop_time is not null and stop_time < now() - interval '1 hour') \
             or (status in ('cancelled', 'expired') and updated_at < now() - interval '1 hour'))",
    )
    .execute(pool)
    .await
    .map_err(internal)?;

    let d_gs = sqlx::query(
        "delete from tmu.ground_stops \
         where published_at is null \
           and ((tmu.ground_stop_until_ts(created_at, until) is not null \
                 and tmu.ground_stop_until_ts(created_at, until) < now() - interval '1 hour') \
             or (status in ('cancelled', 'expired') and updated_at < now() - interval '1 hour'))",
    )
    .execute(pool)
    .await
    .map_err(internal)?;

    // Programs have no status; they're just removed an hour after their scheduled end.
    let d_pgm = sqlx::query(
        "delete from tmu.programs \
         where active_until is not null and active_until < now() - interval '1 hour'",
    )
    .execute(pool)
    .await
    .map_err(internal)?;

    // GDPs — expire once the window (anchored to publish/creation) has passed, then delete
    // after the grace hour. Slots cascade on the delete.
    const GDP_END: &str =
        "tmu.gdp_end_ts(coalesce(published_at, created_at), start_time, end_time)";
    let e_gdp = sqlx::query(&format!(
        "update tmu.gdp set status = 'expired' \
         where status in ('draft', 'published') and {GDP_END} is not null and {GDP_END} < now()"
    ))
    .execute(pool)
    .await
    .map_err(internal)?;

    let d_gdp = sqlx::query(&format!(
        "delete from tmu.gdp \
         where published_at is null \
           and (({GDP_END} is not null and {GDP_END} < now() - interval '1 hour') \
             or (status in ('cancelled', 'expired') and updated_at < now() - interval '1 hour'))"
    ))
    .execute(pool)
    .await
    .map_err(internal)?;

    // 3. Advisories (#537). Cancelled rather than deleted or soft-dismissed: the docs already state
    // the rule — "A published advisory is never edited — it is cancelled and reissued" — and
    // cancelling is what fires the `adv_cancel` job, so the standing Discord post gets corrected
    // instead of being left asserting a restriction that has lapsed.
    //
    // Two reasons an advisory stops being current, kept as separate statements because they are
    // genuinely different facts rather than one rule with a branch.

    // (a) Its own window has passed, plus a grace period. `GRACE` is the "about 30 minutes" the
    // request asked for; the pass runs every `TMU_CLEANUP_INTERVAL`, so the real removal lands in
    // [30, 35) minutes. Only advisories that actually carry a window are touched — a NULL `valid_to`
    // means "no window", and must not read as "infinitely overdue".
    // Both statements run in one transaction with the Discord corrections they imply, so a crash can
    // never leave an advisory cancelled here while its post still stands there.
    let mut tx = pool.begin().await.map_err(internal)?;
    let mut cancelled: Vec<String> = sqlx::query_scalar(
        "update tmu.advisories set status = 'cancelled' \
         where status in ('draft', 'published') \
           and valid_to is not null \
           and valid_to < now() - interval '30 minutes' \
         returning id",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    // (b) The program it was generated from is over. `cancel_program_advisory_tx` only ever runs on a
    // *user* revising or cancelling a program, so a GDP or ground stop that simply timed out left its
    // advisory `published` forever — a real pre-existing bug, folded in here because this is the pass
    // that would otherwise expire some advisories and not these.
    //
    // Safe to run after the deletes above despite `gdp_id`/`ground_stop_id` being
    // `on delete set null`: both deletes require `published_at is null`, and an advisory is only
    // generated for a *published* program, so no program with an advisory is reachable by them.
    cancelled.extend(
        sqlx::query_scalar::<_, String>(
            "update tmu.advisories a set status = 'cancelled' \
             where a.status in ('draft', 'published') \
               and (exists (select 1 from tmu.gdp g \
                             where g.id = a.gdp_id \
                               and g.status in ('expired', 'cancelled')) \
                 or exists (select 1 from tmu.ground_stops s \
                             where s.id = a.ground_stop_id \
                               and s.status in ('expired', 'cancelled'))) \
             returning a.id",
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(internal)?,
    );

    // Withdraw what was posted, beside the post it corrects (#537 review). The cancellation is only
    // worth anything if Discord stops asserting the advisory, and `adv_cancel` is otherwise enqueued
    // by nothing but the manual cancel handler. The channel is the one the publish went to, not one
    // re-derived from today's facility map — the reason `handlers::tmu::cancel_advisory` gives. The
    // `adv_publish` record is also what answers *whether* anything was posted: a draft never has one,
    // and neither does an advisory published while Discord was unconfigured.
    for id in &cancelled {
        let Some(channel_id) =
            crate::repos::integration::published_channel_for_advisory(pool, id).await?
        else {
            continue;
        };
        let Some(adv) = get_advisory_tx(&mut tx, id).await? else {
            continue;
        };
        let job = crate::advisory::cancel_job_payload(&channel_id, &adv);
        crate::repos::integration::enqueue_job(
            &mut tx,
            "adv_cancel",
            &job,
            Some("advisory"),
            Some(id),
        )
        .await?;
    }
    tx.commit().await.map_err(internal)?;

    Ok(CleanupStats {
        expired: e_tmi.rows_affected() + e_gs.rows_affected() + e_gdp.rows_affected(),
        deleted: d_tmi.rows_affected()
            + d_gs.rows_affected()
            + d_pgm.rows_affected()
            + d_gdp.rows_affected(),
        advisories_cancelled: cancelled.len() as u64,
    })
}

/// Prune published-then-finished traffic-management history past the retention window `before`
/// (kept only so the historical dashboard can replay it). Active (still-published) rows are never
/// pruned. Called from the stats compaction job on the same horizon as the position time-series.
pub async fn prune_history(pool: &PgPool, before: DateTime<Utc>) -> Result<u64, ApiError> {
    let mut total = 0u64;
    for table in ["tmu.tmis", "tmu.ground_stops", "tmu.gdp"] {
        let res = sqlx::query(&format!(
            "delete from {table} where published_at is not null \
               and status in ('expired', 'cancelled') \
               and coalesce(ended_at, updated_at) < $1"
        ))
        .bind(before)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
        total += res.rows_affected();
    }
    Ok(total)
}

// --- advisories (ADVZY documents, #457) ---

const ADVISORY_SELECT: &str = "select id, facility, issued_day, number, kind, body, structured, \
    decoded, status, valid_from, valid_to, published_at, created_at from tmu.advisories";

/// Take the next advisory number for `facility` on today's **Zulu** day.
///
/// vATCSCC numbers advisories per issuing facility per day (`vATCSCC ADVZY 002`), and the number is
/// part of the document's identity — so it is allocated here and stored, not computed at render time.
/// Zulu because that is how the wider network numbers them and how every other time in this domain is
/// expressed; a server in another timezone must not roll the sequence at a different moment.
///
/// Runs in the caller's transaction behind a transaction-scoped advisory lock keyed on the facility
/// and day, so two racing allocations serialise rather than both reading the same maximum. The lock
/// releases when the transaction ends, however it ends.
///
/// A row lock cannot do this job: `select max(...) ... for update` is rejected outright by Postgres
/// ("FOR UPDATE is not allowed with aggregate functions"), and locking the current top row would
/// protect nothing on the first allocation of the day, when there is no row to lock. Same
/// `pg_advisory_xact_lock` pattern `repos::faa_surface_seed` uses for seed-once-per-airport.
///
/// The unique constraint on `(facility, issued_day, number)` is the backstop if one ever slips past.
async fn allocate_advisory_number(
    tx: &mut Transaction<'_, Postgres>,
    facility: &str,
    day: chrono::NaiveDate,
) -> Result<i32, ApiError> {
    sqlx::query("select pg_advisory_xact_lock(hashtext($1))")
        .bind(format!("tmu.advisory:{facility}:{day}"))
        .execute(&mut **tx)
        .await
        .map_err(|_| ApiError::Internal)?;

    sqlx::query_scalar::<_, Option<i32>>(
        "select max(number) from tmu.advisories where facility = $1 and issued_day = $2",
    )
    .bind(facility)
    .bind(day)
    .fetch_one(&mut **tx)
    .await
    .map(|max| max.unwrap_or(0) + 1)
    .map_err(|_| ApiError::Internal)
}

/// Whether a create or edit can derive its document from `structured` — i.e. whether `body` is
/// redundant for this request.
///
/// [`advisory_body`] and `handlers::tmu::create_advisory`'s validation must agree on this, so they
/// share one statement of it rather than each testing `kind` for itself (#503).
///
/// The sharing is the load-bearing part, and it is worth being precise about why. If the two *drift* —
/// the handler treating any `structured` request as needing no body while this still renders only
/// `reroute` — then a structured advisory of an unrenderable kind passes validation with an empty body,
/// derives nothing, and **stores an empty document**: the #499 bug. Verified by mutation: making that
/// one-sided change turns `a_structured_create_of_an_underivable_kind_still_needs_a_body` from 400 to
/// 200. Changing the rule *here* stays safe, because both callers move with it.
pub(crate) fn derives_body(kind: &str, structured: Option<&serde_json::Value>) -> bool {
    structured.is_some() && renders_body(kind)
}

/// The kinds [`advisory_body`] has a renderer for.
///
/// One list on purpose. `derives_body` decides whether a create must supply a body (#503) and
/// `advisory_body` decides whether one gets rendered (#461); the two drifting apart is what rejects a
/// create that would have rendered fine, or stores a kind whose document nobody produced. #461 adding
/// GDP and Ground Stop without this is exactly that drift.
fn renders_body(kind: &str) -> bool {
    matches!(
        kind,
        crate::models::ADVISORY_KIND_REROUTE
            | crate::models::ADVISORY_KIND_GDP
            | crate::models::ADVISORY_KIND_GROUND_STOP
    )
}

/// The body to store for an advisory.
///
/// A **typed** advisory's document is re-derived from its fields rather than trusted from the
/// client, mirroring the rule a structured TMI follows (`models::UpdateTmiRequest`): the fields are
/// the source of truth and the document is their rendering, so the two can never drift. A **raw**
/// advisory — one with no `structured` — keeps the text it was given, byte for byte. That is what
/// makes a raw advisory and a structured one of the same content render identically (#458): the
/// same function produced both.
///
/// An unknown `kind` also passes the body through untouched, which is how `kind` stays open for
/// the types #437 has not reached yet without this becoming a dispatch table that must be edited in
/// lockstep. #437's three types — reroute (#458), GDP and Ground Stop (#461) — are all rendered;
/// the tests' `UNRENDERED_KIND` is not, which is what keeps the clearing-rule cases independent of
/// this table.
/// `None` means "nothing to derive" — the caller keeps whatever body it already had in hand. That is
/// deliberately distinct from `Some(String::new())`: on an edit the body is written through
/// `coalesce`, so a derived empty string would blank the stored document, while `None` leaves it be.
fn advisory_body(
    kind: &str,
    structured: Option<&serde_json::Value>,
    ident: &crate::advisory::AdvisoryIdent,
) -> Result<Option<String>, ApiError> {
    if !derives_body(kind, structured) {
        return Ok(None);
    }
    let Some(value) = structured else {
        return Ok(None);
    };
    // One arm per rendered type; anything else falls through to `None` and keeps the body it was
    // given, which is what holds `kind` open for the types #437 has not reached yet.
    match kind {
        crate::models::ADVISORY_KIND_REROUTE => {
            let parsed: crate::models::RerouteAdvisory =
                serde_json::from_value(value.clone()).map_err(|_| ApiError::BadRequest)?;
            Ok(Some(crate::advisory::render_reroute(&parsed, ident)))
        }
        crate::models::ADVISORY_KIND_GDP => {
            let parsed: crate::models::GdpAdvisory =
                serde_json::from_value(value.clone()).map_err(|_| ApiError::BadRequest)?;
            Ok(Some(crate::advisory::render_gdp(&parsed, ident)))
        }
        crate::models::ADVISORY_KIND_GROUND_STOP => {
            let parsed: crate::models::GroundStopAdvisory =
                serde_json::from_value(value.clone()).map_err(|_| ApiError::BadRequest)?;
            Ok(Some(crate::advisory::render_ground_stop(&parsed, ident)))
        }
        _ => Ok(None),
    }
}

/// Creates a draft advisory, allocating its number (#457).
///
/// The number is taken at **draft**, so the author sees the number they will issue under while still
/// writing, and [`delete_advisory`] leaves a gap rather than renumbering when one is abandoned.
///
/// A consequence worth naming rather than discovering: the Zulu day is fixed here, at creation. A draft
/// started 23:59Z and published 00:05Z therefore carries the *previous* day's number. That follows from
/// `issued_day` meaning "the day the number was allocated on" (0085's own words) and is self-consistent
/// — but whether vATCSCC numbers a document by the day it was drafted or the day it was issued is a
/// domain question, open on #457. If it is the issue day, the allocation moves to `publish_advisory`
/// and a draft can no longer show its final number.
pub async fn create_advisory(
    pool: &PgPool,
    req: &CreateAdvisoryRequest,
    created_by: &str,
) -> Result<String, ApiError> {
    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    let id = create_advisory_tx(&mut tx, req, created_by, None).await?;
    tx.commit().await.map_err(|_| ApiError::Internal)?;
    Ok(id)
}

/// Which program an advisory was generated from (#508), or `None` for a hand-authored one. Written to
/// `tmu.advisories.gdp_id` / `ground_stop_id` (migration `0088`) so the revise path can find a
/// program's live advisory to cancel it.
#[derive(Debug, Clone, Copy)]
pub(crate) enum AdvisoryProgram<'a> {
    Gdp(&'a str),
    GroundStop(&'a str),
}

/// [`create_advisory`] on a transaction the caller owns.
///
/// Split out for #508: publishing a GDP or Ground Stop generates its advisory in the *same*
/// transaction as the publish and the slot freeze, so that an advisory cannot exist for a program that
/// did not publish, or the reverse. [`allocate_advisory_number`] already takes the transaction and
/// holds its advisory lock for the rest of it — which is why the caller must do any feed or RBS work
/// *before* opening the transaction, not inside it.
pub(crate) async fn create_advisory_tx(
    tx: &mut Transaction<'_, Postgres>,
    req: &CreateAdvisoryRequest,
    created_by: &str,
    program: Option<AdvisoryProgram<'_>>,
) -> Result<String, ApiError> {
    let facility = req.facility.trim().to_ascii_uppercase();
    let day = Utc::now().date_naive();
    let number = allocate_advisory_number(tx, &facility, day).await?;
    // Rendered here rather than in the handler because the number is allocated in this transaction:
    // the document carries it twice (header and TMI ID), and re-deriving it outside would be a
    // second answer to a question the database has already settled.
    let body = advisory_body(
        req.kind.trim(),
        req.structured.as_ref(),
        &crate::advisory::AdvisoryIdent {
            facility: facility.clone(),
            number,
            issued_day: day,
            signed_at: Utc::now(),
        },
    )?
    .unwrap_or_else(|| req.body.trim().to_string());
    let (gdp_id, ground_stop_id) = match program {
        Some(AdvisoryProgram::Gdp(id)) => (Some(id), None),
        Some(AdvisoryProgram::GroundStop(id)) => (None, Some(id)),
        None => (None, None),
    };
    let id = sqlx::query_scalar::<_, String>(
        "insert into tmu.advisories \
         (facility, issued_day, number, kind, body, structured, decoded, created_by, \
          gdp_id, ground_stop_id, valid_from, valid_to) \
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) returning id",
    )
    .bind(&facility)
    .bind(day)
    .bind(number)
    .bind(req.kind.trim())
    .bind(&body)
    .bind(req.structured.as_ref().map(sqlx::types::Json))
    .bind(req.decoded.as_deref())
    .bind(created_by)
    .bind(gdp_id)
    .bind(ground_stop_id)
    // Stored verbatim from the request, never derived from `body`. The printed period stays the
    // document's own text; this is the window a job may act on (#537).
    .bind(req.valid_from)
    .bind(req.valid_to)
    .fetch_one(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(id)
}

/// Cancels the live (draft or published) advisory generated from `program`, returning how many were
/// cancelled.
///
/// #461 settled that an advisory is cancelled and reissued rather than rewritten, so revising a
/// published program cancels its current advisory and issues a new one in the same transaction. In
/// practice this cancels at most one row — there is only ever one live advisory per program — but it is
/// written as a set operation rather than asserting that, because an unexpected second row should be
/// retired too, not left live alongside the new one.
pub(crate) async fn cancel_program_advisory_tx(
    tx: &mut Transaction<'_, Postgres>,
    program: AdvisoryProgram<'_>,
) -> Result<u64, ApiError> {
    let (column, id) = match program {
        AdvisoryProgram::Gdp(id) => ("gdp_id", id),
        AdvisoryProgram::GroundStop(id) => ("ground_stop_id", id),
    };
    // `column` is one of two internal literals, never user input.
    let sql = format!(
        "update tmu.advisories set status = 'cancelled' \
         where {column} = $1 and status in ('draft', 'published')"
    );
    Ok(sqlx::query(&sql)
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(|_| ApiError::Internal)?
        .rows_affected())
}

/// [`get_advisory`] inside a caller's transaction.
///
/// Needed because publish and cancel read the row back *after* updating it and *before* committing,
/// to build the Discord payload from what was actually written rather than from what the caller
/// hoped was written (VATUSA/OIS#459). Reading on the pool there would see the pre-update row.
pub async fn get_advisory_tx(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
) -> Result<Option<AdvisoryBody>, ApiError> {
    sqlx::query_as::<_, AdvisoryBody>(&format!("{ADVISORY_SELECT} where id = $1"))
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| ApiError::Internal)
}

pub async fn get_advisory(pool: &PgPool, id: &str) -> Result<Option<AdvisoryBody>, ApiError> {
    sqlx::query_as::<_, AdvisoryBody>(&format!("{ADVISORY_SELECT} where id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// Newest first, which for advisories means by the number they were issued under.
pub async fn list_advisories(pool: &PgPool) -> Result<Vec<AdvisoryBody>, ApiError> {
    sqlx::query_as::<_, AdvisoryBody>(&format!(
        "{ADVISORY_SELECT} order by issued_day desc, number desc"
    ))
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Edits a draft. A published advisory is a document that went out; it is cancelled and reissued
/// rather than rewritten.
pub async fn update_advisory(
    pool: &PgPool,
    id: &str,
    req: &UpdateAdvisoryRequest,
) -> Result<bool, ApiError> {
    // A structured edit re-derives the document, the same way a create does, so an edited advisory
    // cannot end up showing fields it no longer has. The identity it renders under comes off the
    // row — the number was settled when the draft was created.
    //
    // The raw-edit half of this — a body edit with no new fields leaving a stale `structured` behind
    // — was the gap #458 deliberately left open and filed as #488; `clear_breakdown` below is that
    // fix, so the two halves now meet here.
    let body = match req.structured.as_ref() {
        None => req.body.clone(),
        Some(structured) => {
            let row = sqlx::query_as::<_, (String, i32, chrono::NaiveDate, String)>(
                "select facility, number, issued_day, kind from tmu.advisories \
                 where id = $1 and status = 'draft'",
            )
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|_| ApiError::Internal)?;
            let Some((facility, number, issued_day, current_kind)) = row else {
                return Ok(false);
            };
            let kind = req.kind.as_deref().unwrap_or(&current_kind);
            advisory_body(
                kind.trim(),
                Some(structured),
                &crate::advisory::AdvisoryIdent {
                    facility,
                    number,
                    issued_day,
                    signed_at: Utc::now(),
                },
            )?
            // Nothing rendered for this kind, so the edit's own body stands — importantly `None`
            // when it supplied none, which leaves the stored document untouched.
            .or_else(|| req.body.clone())
        }
    };

    // A raw body edit invalidates the breakdown. For a structured advisory the `body` is *rendered
    // from* `structured`, so new prose with no new fields leaves the stored breakdown describing a
    // document that is no longer there — and anything reading it then gets a confident wrong answer
    // rather than nothing. Clearing beats coalescing, which is the same conclusion and the same shape
    // `update_tmi` reached for TMIs (#452); advisories arrived after that fix and never got it (#488).
    //
    // `decoded` is where the two paths genuinely differ, so the SQL below cannot be copied across
    // verbatim. `UpdateTmiRequest` has no `decoded` field — it is derived from `structured` inside
    // `update_tmi`, so clearing it there can never discard anything a caller sent. Here the caller
    // supplies it, and a decoding sent *with* a new body describes the new body: it is not stale, so
    // the clear must yield to it rather than overwrite it.
    // And a **new breakdown with no decoding sent** is the mirror case (#499). The stored decoding
    // described the breakdown that has just been replaced, so keeping it is the same confident wrong
    // answer in the other direction — a LAX-SFO breakdown carrying a JFK-BOS decoding.
    //
    // It clears rather than re-deriving, which is the choice worth recording. `update_tmi` re-derives
    // (`render_english`), and the analogue here would be a decoding renderer for an advisory — but
    // none exists, and nothing specifies what it would say that `body` does not, because `body` *is*
    // the rendered document. `AdvisoryBody.decoded` carries no doc comment at all: 0085 took the
    // `structured`/`decoded` pair from `tmu.tmis`'s shape, where `decoded` expands a terse NTML line
    // into the prose an advisory already is. So null is honest here and a stale sentence is not.
    let clear_breakdown = req.structured.is_none() && req.body.is_some();
    let stale_decoding = req.structured.is_some() && req.decoded.is_none();
    let result = sqlx::query(
        "update tmu.advisories set \
            kind = coalesce($2, kind), \
            body = coalesce($3, body), \
            structured = case when $6 then null else coalesce($4, structured) end, \
            decoded = case when $6 then $5 when $7 then null else coalesce($5, decoded) end \
         where id = $1 and status = 'draft'",
    )
    .bind(id)
    .bind(req.kind.as_deref())
    .bind(body.as_deref())
    .bind(req.structured.as_ref().map(sqlx::types::Json))
    .bind(req.decoded.as_deref())
    .bind(clear_breakdown)
    .bind(stale_decoding)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

/// Publishes a draft. Returns false if it was not a draft (or is absent).
///
/// Takes the caller's transaction so the Discord post is enqueued atomically with the publish
/// (VATUSA/OIS#459) — a post must not exist for an advisory that did not publish, and an advisory
/// must not go live with nothing queued to announce it.
pub async fn publish_advisory(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
    published_by: &str,
) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "update tmu.advisories \
         set status = 'published', published_by = $2, published_at = now() \
         where id = $1 and status = 'draft'",
    )
    .bind(id)
    .bind(published_by)
    .execute(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

/// Cancels a draft or published advisory. Its number is **not** released: a published advisory was
/// issued, and a cancelled draft that once held a number is not worth the ambiguity of reissuing it.
///
/// Transactional for the same reason as [`publish_advisory`]: the correction post is enqueued with
/// the cancellation.
pub async fn cancel_advisory(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "update tmu.advisories set status = 'cancelled' \
         where id = $1 and status in ('draft', 'published')",
    )
    .bind(id)
    .execute(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

/// Deletes an abandoned draft. Returns false only if it is not a draft (or is absent).
///
/// Numbers are allocated at draft so the author can see theirs while writing, which means an abandoned
/// draft would otherwise burn one. Reclaiming happens **for free and only at the top of the sequence**,
/// because [`allocate_advisory_number`] is `max(number) + 1`:
///
/// - delete the newest of `{1, 2}` → rows `{1}`, next allocation is 2 — rewound
/// - delete an older one → rows `{2}`, next allocation is 3 — number 1 stays burned, leaving a gap
///
/// A gap is the accepted cost of allocating at draft (#457), and it costs nothing: the unique
/// constraint is on `(facility, issued_day, number)`, not on the sequence being dense.
///
/// This deliberately does **not** refuse a draft below the top. It used to, and that made an older
/// draft permanently undeletable — the handler answered 409 forever, so the only ways out were to
/// delete the newer draft first or to *cancel* the older one, writing a `cancelled` advisory for a
/// document that was never issued. With two controllers drafting for one facility — the expected case,
/// since numbering is per facility — whoever drafted first could never abandon their draft (#457
/// review).
///
/// A published or cancelled advisory is never deleted here at all: that is what `status = 'draft'`
/// is for.
pub async fn delete_advisory(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    let result = sqlx::query("delete from tmu.advisories where id = $1 and status = 'draft'")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::*;
    use crate::models::NtmlRestriction;

    async fn seed_user(pool: &PgPool) -> String {
        sqlx::query_scalar::<_, String>(
            "insert into identity.users (full_name, display_name) \
             values ('Test User', 'Test User') returning id",
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn draft(pool: &PgPool, facility: &str) -> AdvisoryBody {
        let user = seed_user(pool).await;
        let id = create_advisory(
            pool,
            &CreateAdvisoryRequest {
                facility: facility.to_string(),
                kind: "reroute".to_string(),
                body: "vATCSCC ADVZY".to_string(),
                structured: None,
                decoded: None,
                valid_from: None,
                valid_to: None,
            },
            &user,
        )
        .await
        .unwrap();
        get_advisory(pool, &id).await.unwrap().unwrap()
    }

    /// An advisory patch that changes nothing, for tests to fill one field of — mirroring `patch()`
    /// for TMIs. A test-local helper rather than a `Default` derive, so a `ToSchema` model stays
    /// untouched.
    /// A `kind` no renderer claims, for the cases that are about `advisory_body`'s pass-through
    /// rather than about any one document type.
    ///
    /// Named rather than written inline because it has already gone stale twice: these cases used
    /// `"ground_stop"` until #461 made it a rendered type, at which point they began failing with
    /// `BadRequest` on a payload that was never meant to parse. `AFP` (Airspace Flow Program) is a
    /// real vATCSCC initiative that OIS does not implement, so it is unlikely to be claimed by
    /// accident — and if it ever is, this is the single line to change.
    const UNRENDERED_KIND: &str = "afp";

    fn adv_patch() -> UpdateAdvisoryRequest {
        UpdateAdvisoryRequest {
            kind: None,
            body: None,
            structured: None,
            decoded: None,
        }
    }

    /// A draft carrying a structured breakdown, for the #488 cases below.
    ///
    /// Deliberately **not** `reroute`: since #458 that kind is a typed document whose body is rendered
    /// from `structured`, so this placeholder payload is now rejected outright. These cases are about
    /// the clearing rule itself, which is keyed on the request shape and not on any kind, so they use
    /// [`UNRENDERED_KIND`]. `a_raw_edit_on_a_rendered_advisory_clears_its_breakdown` covers the
    /// rendered kind.
    async fn structured_draft(pool: &PgPool) -> AdvisoryBody {
        let user = crate::scope_test_support::seed_user(pool).await;
        let id = create_advisory(
            pool,
            &CreateAdvisoryRequest {
                facility: "DCC".to_string(),
                kind: UNRENDERED_KIND.to_string(),
                body: "vATCSCC ADVZY 001 REROUTE".to_string(),
                structured: Some(serde_json::json!({"routes": [{"from": "JFK", "to": "BOS"}]})),
                decoded: Some("JFK to BOS reroute".to_string()),
                valid_from: None,
                valid_to: None,
            },
            &user,
        )
        .await
        .unwrap();
        get_advisory(pool, &id).await.unwrap().unwrap()
    }

    /// #461 AC2: a `gdp` advisory's body is **rendered**, which is what proves the dispatch arm in
    /// [`advisory_body`] is wired and not merely written.
    ///
    /// `advisory.rs` already pins the document itself against `fixtures/gdp-reference.json`. What
    /// only a round-trip can show is that creating a GDP reaches that renderer at all: delete the
    /// `ADVISORY_KIND_GDP` arm and every renderer test stays green while a real GDP stores the raw
    /// body it was handed. So this asserts the supplied body is *replaced*, not merely that the
    /// stored one looks plausible.
    #[sqlx::test]
    async fn a_gdp_advisory_body_is_rendered_from_its_fields(pool: PgPool) {
        let user = crate::scope_test_support::seed_user(&pool).await;
        let id = create_advisory(
            &pool,
            &CreateAdvisoryRequest {
                facility: "DCC".to_string(),
                kind: crate::models::ADVISORY_KIND_GDP.to_string(),
                body: "THIS RAW TEXT MUST NOT SURVIVE".to_string(),
                structured: Some(serde_json::json!({
                    "header": "CDM GROUND DELAY PROGRAM",
                    "element": "JFK/ZNY",
                    "control_element": "JFK",
                    "element_type": "APT",
                    "adl_time": "1349Z",
                    "delay_assignment_mode": "DAS",
                    "arrivals_estimated_for": "14/1415Z - 14/2315Z",
                    "cumulative_program_period": "14/1415Z - 14/2315Z",
                    "program_rate": "40/40/40/30/25/20/20/36/54",
                    "pop_up_factor": "MEDIUM",
                    "flights_included": ["1stTier", "CZY"],
                    "departure_scope": "1200",
                    "impacting_condition": "WEATHER / THUNDERSTORMS",
                    "comments": "ADVZY 002 SUPERSEDES ADVZY 001",
                    "period": "141415-142315",
                })),
                decoded: None,
                valid_from: None,
                valid_to: None,
            },
            &user,
        )
        .await
        .unwrap();

        let stored = get_advisory(&pool, &id).await.unwrap().unwrap();
        assert!(
            !stored.body.contains("THIS RAW TEXT MUST NOT SURVIVE"),
            "a rendered kind must not keep the body it was handed: {}",
            stored.body
        );
        for line in [
            "CTL ELEMENT: JFK",
            "ELEMENT TYPE: APT",
            "DELAY ASSIGNMENT MODE: DAS",
            "PROGRAM RATE: 40/40/40/30/25/20/20/36/54",
            "FLT INCL: 1stTier",
            "141415-142315",
        ] {
            assert!(
                stored.body.contains(line),
                "missing {line:?}: {}",
                stored.body
            );
        }
        // The header's element slot is the control element, not the issuing facility (`DCC`),
        // which a reroute would print here instead.
        assert!(
            stored.body.starts_with("vATCSCC ADVZY 001 JFK/ZNY "),
            "header must carry the control element: {}",
            stored.body
        );
        assert!(
            !stored.body.contains("DCC"),
            "the issuing facility has no place in a GDP document: {}",
            stored.body
        );
    }

    /// The Ground Stop half of #461 AC2, for the reason the GDP case gives: delete the
    /// `ADVISORY_KIND_GROUND_STOP` arm and every renderer test stays green while a real ground stop
    /// stores the raw body it was handed.
    #[sqlx::test]
    async fn a_ground_stop_advisory_body_is_rendered_from_its_fields(pool: PgPool) {
        let user = crate::scope_test_support::seed_user(&pool).await;
        let id = create_advisory(
            &pool,
            &CreateAdvisoryRequest {
                facility: "DCC".to_string(),
                kind: crate::models::ADVISORY_KIND_GROUND_STOP.to_string(),
                body: "THIS RAW TEXT MUST NOT SURVIVE".to_string(),
                structured: Some(serde_json::json!({
                    "header": "CDM GROUND STOP",
                    "element": "DFW/ZFW",
                    "control_element": "DFW",
                    "element_type": "APT",
                    "adl_time": "1354Z",
                    "ground_stop_period": "14/1430Z - 14/1630Z",
                    "cumulative_program_period": "14/1430Z - 14/1630Z",
                    "flights_included": ["(Manual) ZHU ZJX ZMA ZME ZTL"],
                    "current_delays": "1240/414/81",
                    "previous_delays": "636/211/70",
                    "new_delays": "1876/625/151",
                    "probability_of_extension": "MEDIUM",
                    "impacting_condition": "EQUIPMENT / STARS",
                    "comments": "BLAH",
                    "period": "141430-141630",
                })),
                decoded: None,
                valid_from: None,
                valid_to: None,
            },
            &user,
        )
        .await
        .unwrap();

        let stored = get_advisory(&pool, &id).await.unwrap().unwrap();
        assert!(
            !stored.body.contains("THIS RAW TEXT MUST NOT SURVIVE"),
            "a rendered kind must not keep the body it was handed: {}",
            stored.body
        );
        for line in [
            "CTL ELEMENT: DFW",
            "GROUND STOP PERIOD: 14/1430Z - 14/1630Z",
            "FLT INCL: (Manual) ZHU ZJX ZMA ZME ZTL",
            "CURRENT TOTAL, MAXIMUM, AVERAGE DELAYS: 1240/414/81",
            "NEW TOTAL, MAXIMUM, AVERAGE DELAYS: 1876/625/151",
        ] {
            assert!(
                stored.body.contains(line),
                "missing {line:?}: {}",
                stored.body
            );
        }
    }

    /// #488 AC1. `update_advisory` used to coalesce `structured`, so editing a raw body left the old
    /// breakdown behind, describing a document that no longer existed. `body` for a structured
    /// advisory is rendered *from* `structured`, so the two are one fact expressed twice; a raw edit
    /// breaks that and the breakdown has to go rather than silently disagree.
    #[sqlx::test]
    async fn a_raw_body_edit_clears_the_breakdown(pool: PgPool) {
        let before = structured_draft(&pool).await;
        assert!(before.structured.is_some(), "fixture must start with one");

        assert!(
            update_advisory(
                &pool,
                &before.id,
                &UpdateAdvisoryRequest {
                    body: Some("vATCSCC ADVZY 001 FREE TEXT".to_string()),
                    ..adv_patch()
                },
            )
            .await
            .unwrap()
        );

        let after = get_advisory(&pool, &before.id).await.unwrap().unwrap();
        assert_eq!(after.body, "vATCSCC ADVZY 001 FREE TEXT");
        assert!(
            after.structured.is_none(),
            "stale breakdown survived a raw edit"
        );
        assert!(
            after.decoded.is_none(),
            "stale decoding survived a raw edit"
        );
    }

    /// A raw edit clears the *stale* breakdown, not a decoding the caller sent in the same patch.
    ///
    /// `update_tmi`'s clear covers `decoded` too, and copying that across discarded a supplied value:
    /// `UpdateTmiRequest` has no `decoded` (it is derived from `structured`), so the clear there can
    /// only ever null something already null. `UpdateAdvisoryRequest` does have one, and
    /// `CreateAdvisoryRequest` takes `structured` and `decoded` as independent optionals — so a raw
    /// advisory with a hand-written decoding is a state the API lets you build, and editing its body
    /// silently wiped the decoding while reporting success.
    #[sqlx::test]
    async fn a_decoding_supplied_with_the_new_body_is_kept(pool: PgPool) {
        let before = structured_draft(&pool).await;

        assert!(
            update_advisory(
                &pool,
                &before.id,
                &UpdateAdvisoryRequest {
                    body: Some("vATCSCC ADVZY 001 FREE TEXT".to_string()),
                    decoded: Some("hand-written decoding".to_string()),
                    ..adv_patch()
                },
            )
            .await
            .unwrap()
        );

        let after = get_advisory(&pool, &before.id).await.unwrap().unwrap();
        assert!(
            after.structured.is_none(),
            "the breakdown no longer describes this body and must still go"
        );
        assert_eq!(
            after.decoded.as_deref(),
            Some("hand-written decoding"),
            "a decoding sent with the new body describes it, so it is not stale"
        );
    }

    /// Where #458 and #488 meet, and the case neither could write alone: a `reroute`'s body is
    /// *rendered from* its breakdown, so a hand-edited raw body is precisely when the stored breakdown
    /// stops describing the document. The clear has to fire on the rendered kind too — it is keyed on
    /// the request shape, not on the kind, and this pins that.
    #[sqlx::test]
    async fn a_raw_edit_on_a_rendered_advisory_clears_its_breakdown(pool: PgPool) {
        let user = crate::scope_test_support::seed_user(&pool).await;
        let id = create_advisory(
            &pool,
            &CreateAdvisoryRequest {
                facility: "DCC".to_string(),
                kind: "reroute".to_string(),
                body: String::new(),
                structured: Some(reroute_structured()),
                decoded: None,
                valid_from: None,
                valid_to: None,
            },
            &user,
        )
        .await
        .unwrap();
        let before = get_advisory(&pool, &id).await.unwrap().unwrap();
        assert!(
            before.structured.is_some() && before.body.contains("NO_J75_3_PARTIAL"),
            "fixture must start as a rendered reroute: {}",
            before.body
        );

        assert!(
            update_advisory(
                &pool,
                &id,
                &UpdateAdvisoryRequest {
                    body: Some("vATCSCC ADVZY 001 REROUTE CANCELLED BY HAND".to_string()),
                    ..adv_patch()
                },
            )
            .await
            .unwrap()
        );

        let after = get_advisory(&pool, &id).await.unwrap().unwrap();
        assert_eq!(after.body, "vATCSCC ADVZY 001 REROUTE CANCELLED BY HAND");
        assert!(
            after.structured.is_none(),
            "the rendered breakdown no longer describes this body and must go"
        );
    }

    /// #499. A new breakdown with no decoding sent must not keep the old decoding, which described
    /// the breakdown that was just replaced — the same stale-pair defect #488 fixed, on the other
    /// axis. This is the case #488's own `supplying_a_breakdown_replaces_it` could not catch,
    /// because it supplies `decoded` alongside `structured` and so never exercises the omission.
    #[sqlx::test]
    async fn a_new_breakdown_with_no_decoding_clears_the_old_one(pool: PgPool) {
        let before = structured_draft(&pool).await;
        assert!(
            before.decoded.is_some(),
            "fixture must start with a decoding"
        );

        assert!(
            update_advisory(
                &pool,
                &before.id,
                &UpdateAdvisoryRequest {
                    structured: Some(serde_json::json!({"routes": [{"from": "LAX", "to": "SFO"}]})),
                    ..adv_patch()
                },
            )
            .await
            .unwrap()
        );

        let after = get_advisory(&pool, &before.id).await.unwrap().unwrap();
        let structured = after.structured.expect("the new breakdown must be stored");
        assert_eq!(structured["routes"][0]["from"], "LAX");
        assert!(
            after.decoded.is_none(),
            "a decoding describing the replaced breakdown survived"
        );
    }

    /// The reason this cannot be a blanket clear on any structured edit: a decoding sent *with* the
    /// new breakdown describes that breakdown, so it is not stale and must be stored.
    #[sqlx::test]
    async fn a_decoding_sent_with_the_new_breakdown_is_kept(pool: PgPool) {
        let before = structured_draft(&pool).await;

        assert!(
            update_advisory(
                &pool,
                &before.id,
                &UpdateAdvisoryRequest {
                    structured: Some(serde_json::json!({"routes": [{"from": "LAX", "to": "SFO"}]})),
                    decoded: Some("LAX to SFO reroute".to_string()),
                    ..adv_patch()
                },
            )
            .await
            .unwrap()
        );

        let after = get_advisory(&pool, &before.id).await.unwrap().unwrap();
        assert_eq!(after.decoded.as_deref(), Some("LAX to SFO reroute"));
    }

    /// An edit touching neither field leaves the decoding alone — a kind-only edit is not a reason
    /// to drop it, which is what makes the guard `structured.is_some()` rather than `decoded.is_none()`.
    #[sqlx::test]
    async fn a_kind_only_edit_keeps_the_decoding(pool: PgPool) {
        let before = structured_draft(&pool).await;

        assert!(
            update_advisory(
                &pool,
                &before.id,
                &UpdateAdvisoryRequest {
                    kind: Some("gdp".to_string()),
                    ..adv_patch()
                },
            )
            .await
            .unwrap()
        );

        let after = get_advisory(&pool, &before.id).await.unwrap().unwrap();
        assert_eq!(after.decoded, before.decoded);
    }

    /// #488 AC2, first half: supplying a breakdown still replaces it. This is why the guard is keyed
    /// on `structured.is_none()` rather than on the body changing at all.
    #[sqlx::test]
    async fn supplying_a_breakdown_replaces_it(pool: PgPool) {
        let before = structured_draft(&pool).await;

        assert!(
            update_advisory(
                &pool,
                &before.id,
                &UpdateAdvisoryRequest {
                    body: Some("vATCSCC ADVZY 001 REROUTE (REVISED)".to_string()),
                    structured: Some(serde_json::json!({"routes": [{"from": "EWR", "to": "ORD"}]})),
                    decoded: Some("EWR to ORD reroute".to_string()),
                    ..adv_patch()
                },
            )
            .await
            .unwrap()
        );

        let after = get_advisory(&pool, &before.id).await.unwrap().unwrap();
        let structured = after.structured.expect("a supplied breakdown must survive");
        assert_eq!(structured["routes"][0]["from"], "EWR");
        assert_eq!(after.decoded.as_deref(), Some("EWR to ORD reroute"));
    }

    /// #488 AC2, second half, and the reason this cannot be a blanket clear: an edit that does not
    /// touch the body must leave the breakdown alone.
    #[sqlx::test]
    async fn editing_neither_leaves_the_breakdown_alone(pool: PgPool) {
        let before = structured_draft(&pool).await;

        assert!(
            update_advisory(
                &pool,
                &before.id,
                &UpdateAdvisoryRequest {
                    // Any kind will do — this case is about the clearing rule, not the type.
                    kind: Some(UNRENDERED_KIND.to_string()),
                    ..adv_patch()
                },
            )
            .await
            .unwrap()
        );

        let after = get_advisory(&pool, &before.id).await.unwrap().unwrap();
        assert_eq!(after.kind, UNRENDERED_KIND);
        assert!(
            after.structured.is_some(),
            "an edit that left the body alone must not clear the breakdown"
        );
        assert!(after.decoded.is_some());
    }

    /// The reference single-segment reroute, as a structured payload.
    fn reroute_structured() -> serde_json::Value {
        serde_json::json!({
            "header": "FCA RQD/FL",
            "name": "NO_J75_3_PARTIAL",
            "impacted_area": "ZDC",
            "reason": "WEATHER / THUNDERSTORMS",
            "include_traffic": "KBOS DEPARTURES TO KMCO",
            "valid": {"basis": "fca_entry_time", "from": "142030", "to": "150230"},
            "facilities_included": "ALL_FLIGHTS",
            "probability_of_extension": "MEDIUM",
            "remarks": null,
            "associated_restrictions": null,
            "modifications": null,
            "routes": {
                "kind": "single",
                "rows": [{
                    "orig": "ZBW", "dest": "MCO",
                    "route": ">GONZZ Q29 DORET DJB J84 SPA J85 TWINS JEFOI SHEMP< BUGGZ4"
                }]
            }
        })
    }

    async fn create(pool: &PgPool, req: CreateAdvisoryRequest) -> AdvisoryBody {
        let user = seed_user(pool).await;
        let id = create_advisory(pool, &req, &user).await.unwrap();
        get_advisory(pool, &id).await.unwrap().unwrap()
    }

    // --- the Reroute document type (VATUSA/OIS#458) ---

    /// A structured advisory's document is rendered from its fields, not taken from the client — so
    /// a client that posts a body contradicting its own fields cannot store the contradiction.
    #[sqlx::test]
    async fn a_structured_reroute_renders_its_own_document(pool: PgPool) {
        let adv = create(
            &pool,
            CreateAdvisoryRequest {
                facility: "DCC".into(),
                kind: "reroute".into(),
                body: "IGNORE ME".into(),
                structured: Some(reroute_structured()),
                decoded: None,
                valid_from: None,
                valid_to: None,
            },
        )
        .await;

        assert!(
            adv.body.starts_with("vATCSCC ADVZY 001 DCC "),
            "{}",
            adv.body
        );
        assert!(adv.body.contains("NAME: NO_J75_3_PARTIAL"));
        assert!(adv.body.contains("ORIG     DEST      ROUTE"));
        assert!(adv.body.contains("TMI ID: RRDCC001"));
        assert!(
            !adv.body.contains("IGNORE ME"),
            "the posted body was trusted"
        );
    }

    /// The header and the TMI ID both carry the number, and it is the one the database allocated —
    /// not a second answer computed at render time.
    #[sqlx::test]
    async fn the_document_carries_the_allocated_number_in_both_places(pool: PgPool) {
        create(
            &pool,
            CreateAdvisoryRequest {
                facility: "DCC".into(),
                kind: "reroute".into(),
                body: "x".into(),
                structured: Some(reroute_structured()),
                decoded: None,
                valid_from: None,
                valid_to: None,
            },
        )
        .await;
        let second = create(
            &pool,
            CreateAdvisoryRequest {
                facility: "DCC".into(),
                kind: "reroute".into(),
                body: "x".into(),
                structured: Some(reroute_structured()),
                decoded: None,
                valid_from: None,
                valid_to: None,
            },
        )
        .await;

        assert_eq!(second.number, 2);
        assert!(second.body.starts_with("vATCSCC ADVZY 002 DCC "));
        assert!(second.body.contains("TMI ID: RRDCC002"));
    }

    /// #458 AC3: a raw advisory keeps its text byte for byte, and a raw advisory carrying the
    /// rendered document is indistinguishable from the structured one that produced it.
    #[sqlx::test]
    async fn a_raw_reroute_is_stored_verbatim_and_matches_the_structured_form(pool: PgPool) {
        let structured = create(
            &pool,
            CreateAdvisoryRequest {
                facility: "DCC".into(),
                kind: "reroute".into(),
                body: "x".into(),
                structured: Some(reroute_structured()),
                decoded: None,
                valid_from: None,
                valid_to: None,
            },
        )
        .await;

        // Same document, typed as raw text into a *different* facility's advisory so it gets its
        // own number — then compare everything below the header, which is what the author wrote.
        let raw = create(
            &pool,
            CreateAdvisoryRequest {
                facility: "DCC".into(),
                kind: "reroute".into(),
                body: structured.body.clone(),
                structured: None,
                decoded: None,
                valid_from: None,
                valid_to: None,
            },
        )
        .await;

        assert!(raw.structured.is_none(), "a raw advisory has no breakdown");
        assert_eq!(
            raw.body, structured.body,
            "a raw advisory is stored exactly as typed"
        );
    }

    /// An unknown kind is passed through untouched, so `kind` stays open for #461's types without
    /// this becoming a dispatch table that has to be edited in lockstep.
    #[sqlx::test]
    async fn an_unknown_kind_keeps_whatever_body_it_was_given(pool: PgPool) {
        let adv = create(
            &pool,
            CreateAdvisoryRequest {
                facility: "DCC".into(),
                kind: UNRENDERED_KIND.into(),
                body: "SOME OTHER DOCUMENT".into(),
                structured: Some(reroute_structured()),
                decoded: None,
                valid_from: None,
                valid_to: None,
            },
        )
        .await;
        assert_eq!(adv.body, "SOME OTHER DOCUMENT");
    }

    /// A structured payload that is not a reroute is a client error, not a silently empty document.
    #[sqlx::test]
    async fn a_malformed_reroute_payload_is_rejected(pool: PgPool) {
        let user = seed_user(&pool).await;
        let err = create_advisory(
            &pool,
            &CreateAdvisoryRequest {
                facility: "DCC".into(),
                kind: "reroute".into(),
                body: "x".into(),
                structured: Some(serde_json::json!({"nope": true})),
                decoded: None,
                valid_from: None,
                valid_to: None,
            },
            &user,
        )
        .await;
        assert!(matches!(err, Err(ApiError::BadRequest)), "{err:?}");
    }

    /// Editing the fields re-renders the document, so an edited draft cannot keep showing the
    /// values it no longer has.
    #[sqlx::test]
    async fn a_structured_edit_re_renders_the_document(pool: PgPool) {
        let adv = create(
            &pool,
            CreateAdvisoryRequest {
                facility: "DCC".into(),
                kind: "reroute".into(),
                body: "x".into(),
                structured: Some(reroute_structured()),
                decoded: None,
                valid_from: None,
                valid_to: None,
            },
        )
        .await;

        let mut edited = reroute_structured();
        edited["name"] = serde_json::json!("RENAMED_ROUTE");
        assert!(
            update_advisory(
                &pool,
                &adv.id,
                &UpdateAdvisoryRequest {
                    kind: None,
                    body: None,
                    structured: Some(edited),
                    decoded: None,
                },
            )
            .await
            .unwrap()
        );

        let after = get_advisory(&pool, &adv.id).await.unwrap().unwrap();
        assert!(after.body.contains("NAME: RENAMED_ROUTE"), "{}", after.body);
        assert!(!after.body.contains("NO_J75_3_PARTIAL"));
        assert!(
            after.body.contains("TMI ID: RRDCC001"),
            "the number is unchanged by an edit"
        );
    }

    /// A structured edit on a kind this renderer knows nothing about must leave the stored document
    /// alone. The body is written through `coalesce`, so "derived nothing" and "derived an empty
    /// string" are very different answers — the second blanks the document.
    #[sqlx::test]
    async fn a_structured_edit_on_an_unknown_kind_leaves_the_body_alone(pool: PgPool) {
        let adv = create(
            &pool,
            CreateAdvisoryRequest {
                facility: "DCC".into(),
                kind: UNRENDERED_KIND.into(),
                body: "SOME OTHER DOCUMENT".into(),
                structured: None,
                decoded: None,
                valid_from: None,
                valid_to: None,
            },
        )
        .await;

        assert!(
            update_advisory(
                &pool,
                &adv.id,
                &UpdateAdvisoryRequest {
                    kind: None,
                    body: None,
                    structured: Some(serde_json::json!({"anything": 1})),
                    decoded: None,
                },
            )
            .await
            .unwrap()
        );

        let after = get_advisory(&pool, &adv.id).await.unwrap().unwrap();
        assert_eq!(
            after.body, "SOME OTHER DOCUMENT",
            "a structured edit blanked a document it could not render"
        );
    }

    /// #457, AC2: the sequence is per issuing facility, so two facilities numbering on the same day
    /// do not share a counter.
    #[sqlx::test]
    async fn numbers_run_per_facility(pool: PgPool) {
        assert_eq!(draft(&pool, "DCC").await.number, 1);
        assert_eq!(draft(&pool, "DCC").await.number, 2);
        assert_eq!(
            draft(&pool, "ZNY").await.number,
            1,
            "a second facility starts its own run"
        );
        assert_eq!(draft(&pool, "DCC").await.number, 3);
    }

    /// AC3. Two advisories must never take the same number, and the allocator is what guarantees
    /// it — the unique constraint is only the backstop that turns a race into an error instead of a
    /// duplicate identity.
    ///
    /// Eight concurrent creates rather than two: with two, removing `pg_advisory_xact_lock` still
    /// let the test pass roughly one run in five, because the window in which both transactions
    /// read the same `max(number)` is narrow enough to miss (#457 review). Eight makes the race
    /// near-certain, so the test fails every time the lock is gone rather than most of the time.
    #[sqlx::test]
    async fn concurrent_allocations_get_different_numbers(pool: PgPool) {
        const CONCURRENCY: usize = 8;
        let user = seed_user(&pool).await;
        // Built per task rather than cloned: `CreateAdvisoryRequest` is a request model and does not
        // derive `Clone`, which is not worth changing for a test.
        let request = || CreateAdvisoryRequest {
            facility: "DCC".to_string(),
            kind: "reroute".to_string(),
            body: "vATCSCC ADVZY".to_string(),
            structured: None,
            decoded: None,
            valid_from: None,
            valid_to: None,
        };

        // Spawned, not just awaited together: each create needs its own task to contend for a
        // separate pool connection, which is what makes the allocation genuinely concurrent.
        let mut tasks = Vec::new();
        for _ in 0..CONCURRENCY {
            let (pool, req, user) = (pool.clone(), request(), user.clone());
            tasks.push(tokio::spawn(async move {
                create_advisory(&pool, &req, &user).await
            }));
        }

        let mut numbers = Vec::new();
        for task in tasks {
            let id = task
                .await
                .expect("task panicked")
                .expect("every concurrent create must succeed");
            numbers.push(get_advisory(&pool, &id).await.unwrap().unwrap().number);
        }
        numbers.sort_unstable();

        assert_eq!(
            numbers,
            (1..=CONCURRENCY as i32).collect::<Vec<_>>(),
            "concurrent allocations must be a dense 1..=n with no duplicates and no gaps"
        );
    }

    /// AC4. Numbers are taken at draft so the author can see theirs, which means abandoning one must
    /// give it back — but only at the top of the sequence. Releasing a number below the maximum
    /// would leave the gap anyway or need the drafts above it renumbered.
    #[sqlx::test]
    async fn abandoning_the_newest_draft_rewinds_the_sequence(pool: PgPool) {
        let first = draft(&pool, "DCC").await;
        let second = draft(&pool, "DCC").await;
        assert_eq!((first.number, second.number), (1, 2));

        assert!(delete_advisory(&pool, &second.id).await.unwrap());
        assert_eq!(
            draft(&pool, "DCC").await.number,
            2,
            "2 should have come back"
        );
    }

    /// The other half of AC4: a draft that is no longer the newest keeps its number burned, because
    /// the alternative is renumbering something someone is already looking at.
    #[sqlx::test]
    async fn abandoning_an_older_draft_leaves_its_number_burned(pool: PgPool) {
        let first = draft(&pool, "DCC").await;
        let second = draft(&pool, "DCC").await;
        assert_eq!((first.number, second.number), (1, 2));

        assert!(
            delete_advisory(&pool, &first.id).await.unwrap(),
            "an older draft must still be abandonable — refusing it left it undeletable forever"
        );
        assert!(get_advisory(&pool, &first.id).await.unwrap().is_none());

        // The gap is the point: 1 is burned, and the sequence carries on above the survivor.
        assert_eq!(
            draft(&pool, "DCC").await.number,
            3,
            "1 must not be reissued while 2 still holds the top"
        );
    }

    /// AC4, and the line that does not move: a published advisory went out, and a cancelled one is
    /// still a record of what was issued. Neither number is ever handed to something else.
    #[sqlx::test]
    async fn a_published_or_cancelled_number_is_never_reissued(pool: PgPool) {
        let user = seed_user(&pool).await;
        let published = draft(&pool, "DCC").await;
        // Transactional since #459, so the Discord enqueue is atomic with the state change.
        let mut tx = pool.begin().await.unwrap();
        assert!(
            publish_advisory(&mut tx, &published.id, &user)
                .await
                .unwrap()
        );
        tx.commit().await.unwrap();

        assert!(
            !delete_advisory(&pool, &published.id).await.unwrap(),
            "a published advisory must not be deletable"
        );
        let mut tx = pool.begin().await.unwrap();
        assert!(cancel_advisory(&mut tx, &published.id).await.unwrap());
        tx.commit().await.unwrap();
        assert!(!delete_advisory(&pool, &published.id).await.unwrap());

        assert_eq!(
            draft(&pool, "DCC").await.number,
            2,
            "the next draft must not reuse 1"
        );
    }

    fn ntml(element: &str, value: i64) -> NtmlRestriction {
        serde_json::from_value(serde_json::json!({
            "element": element,
            "direction": "arrivals",
            "kind": "MIT",
            "via": "CAMRN",
            "value": value,
        }))
        .unwrap()
    }

    /// A structured TMI to edit. Returns its id and the breakdown it started with.
    async fn structured_tmi(pool: &PgPool) -> (String, NtmlRestriction) {
        let user = seed_user(pool).await;
        let original = ntml("JFK", 20);
        let id = create_tmi(
            pool,
            &CreateTmiRequest {
                requesting: "ZDC".to_string(),
                providing: "ZNY".to_string(),
                restriction: crate::tmi::encode(&original),
                structured: Some(original.clone()),
                start_time: None,
                stop_time: None,
            },
            &user,
        )
        .await
        .unwrap();
        (id, original)
    }

    /// `update_tmi` runs in the caller's transaction since #453, so it can enqueue the corrected
    /// Discord row atomically with the edit. These tests only care that the edit applied, so they
    /// open a transaction, commit it, and hand back what it reported.
    async fn edit(pool: &PgPool, id: &str, req: &UpdateTmiRequest) -> Option<TmiEdit> {
        let mut tx = pool.begin().await.unwrap();
        let edited = update_tmi(&mut tx, id, req).await.unwrap();
        tx.commit().await.unwrap();
        edited
    }

    fn patch() -> UpdateTmiRequest {
        UpdateTmiRequest {
            requesting: None,
            providing: None,
            restriction: None,
            structured: None,
            start_time: None,
            stop_time: None,
        }
    }

    /// #452, and the whole point of the issue: the row must never hold a new raw line beside the old
    /// parsed fields, because "View structured" then answers with a restriction that is not in force.
    #[sqlx::test]
    async fn a_structured_edit_replaces_the_breakdown(pool: PgPool) {
        let (id, original) = structured_tmi(&pool).await;
        let before = get_tmi(&pool, &id).await.unwrap().unwrap();
        assert_eq!(before.decoded, Some(crate::tmi::render_english(&original)));

        // Edit through the fields: 20MIT → 30MIT.
        let edited = ntml("JFK", 30);
        assert!(
            edit(
                &pool,
                &id,
                &UpdateTmiRequest {
                    restriction: Some(crate::tmi::encode(&edited)),
                    structured: Some(edited.clone()),
                    ..patch()
                },
            )
            .await
            .is_some()
        );

        let after = get_tmi(&pool, &id).await.unwrap().unwrap();
        assert_eq!(after.restriction, crate::tmi::encode(&edited));
        assert_eq!(after.decoded, Some(crate::tmi::render_english(&edited)));
        assert_ne!(
            after.decoded, before.decoded,
            "the breakdown must not still be the pre-edit one"
        );
        assert_eq!(after.structured.unwrap().0.value, Some(30));
    }

    /// Editing only the raw text leaves no breakdown that could describe it, so the stored one is
    /// dropped rather than kept — a raw-typed TMI's honest "no breakdown" is better than a wrong one.
    #[sqlx::test]
    async fn a_raw_edit_clears_the_breakdown(pool: PgPool) {
        let (id, _) = structured_tmi(&pool).await;

        assert!(
            edit(
                &pool,
                &id,
                &UpdateTmiRequest {
                    restriction: Some("JFK arrivals via CAMRN 30MIT NO STACKS".to_string()),
                    ..patch()
                },
            )
            .await
            .is_some()
        );

        let after = get_tmi(&pool, &id).await.unwrap().unwrap();
        assert_eq!(after.restriction, "JFK arrivals via CAMRN 30MIT NO STACKS");
        assert!(
            after.structured.is_none(),
            "stale breakdown survived a raw edit"
        );
        assert!(after.decoded.is_none());
    }

    /// The reason this cannot be a blanket clear: an edit that does not touch the restriction must
    /// leave the breakdown alone.
    #[sqlx::test]
    async fn editing_only_the_window_keeps_the_breakdown(pool: PgPool) {
        let (id, original) = structured_tmi(&pool).await;

        assert!(
            edit(
                &pool,
                &id,
                &UpdateTmiRequest {
                    stop_time: Some(chrono::Utc::now() + chrono::Duration::hours(2)),
                    ..patch()
                },
            )
            .await
            .is_some()
        );

        let after = get_tmi(&pool, &id).await.unwrap().unwrap();
        assert!(
            after.structured.is_some(),
            "an unrelated edit dropped the breakdown"
        );
        assert_eq!(after.decoded, Some(crate::tmi::render_english(&original)));
    }

    /// The `restriction`/`structured`/`decoded` split a raw-typed TMI and a structured (form-built)
    /// one leave for the bot's "View structured" reply: a raw TMI has `structured`/`decoded` null,
    /// a structured one has both populated (mirrors `handlers::tmu::create_tmi`'s own pre-processing,
    /// since that derivation happens in the handler, not this repo layer).
    #[sqlx::test]
    async fn get_tmi_reflects_raw_vs_structured_entry(pool: PgPool) {
        let user = seed_user(&pool).await;

        let raw_id = create_tmi(
            &pool,
            &CreateTmiRequest {
                requesting: "ZDC".to_string(),
                providing: "ZNY".to_string(),
                restriction: "ZDC ZNY 20MIT via CAMRN".to_string(),
                structured: None,
                start_time: None,
                stop_time: None,
            },
            &user,
        )
        .await
        .unwrap();
        let raw = get_tmi(&pool, &raw_id).await.unwrap().unwrap();
        assert_eq!(raw.restriction, "ZDC ZNY 20MIT via CAMRN");
        assert!(raw.structured.is_none());
        assert!(raw.decoded.is_none());

        let structured: NtmlRestriction = serde_json::from_value(serde_json::json!({
            "element": "JFK",
            "direction": "arrivals",
            "kind": "MIT",
            "via": "CAMRN",
            "value": 20,
        }))
        .unwrap();
        let encoded = crate::tmi::encode(&structured);
        let structured_id = create_tmi(
            &pool,
            &CreateTmiRequest {
                requesting: "ZDC".to_string(),
                providing: "ZNY".to_string(),
                restriction: encoded.clone(),
                structured: Some(structured),
                start_time: None,
                stop_time: None,
            },
            &user,
        )
        .await
        .unwrap();
        let built = get_tmi(&pool, &structured_id).await.unwrap().unwrap();
        assert_eq!(built.restriction, encoded);
        assert!(built.structured.is_some());
        assert!(built.decoded.is_some());
    }

    // --- #304: deleting a published (kept-for-history) row takes it off the TMU list ---

    async fn raw_tmi(pool: &PgPool, user: &str) -> String {
        create_tmi(
            pool,
            &CreateTmiRequest {
                requesting: "ZDC".to_string(),
                providing: "ZNY".to_string(),
                restriction: "ZDC ZNY 20MIT".to_string(),
                structured: None,
                start_time: None,
                stop_time: None,
            },
            user,
        )
        .await
        .unwrap()
    }

    /// Mark `id` in `table` as published 2h ago and ended (expired) 1h ago, like `run_cleanup` leaves it.
    async fn publish_then_expire(pool: &PgPool, table: &str, id: &str) {
        sqlx::query(&format!(
            "update {table} set status = 'expired', published_at = now() - interval '2 hours' \
             where id = $1"
        ))
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn row_exists(pool: &PgPool, table: &str, id: &str) -> bool {
        sqlx::query_scalar::<_, bool>(&format!(
            "select exists(select 1 from {table} where id = $1)"
        ))
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn listed_tmi_ids(pool: &PgPool) -> Vec<String> {
        list_tmis(pool, &TmiFilters::default())
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect()
    }

    #[sqlx::test]
    async fn deleting_an_expired_published_tmi_removes_it_from_the_list_but_keeps_its_history(
        pool: PgPool,
    ) {
        let user = seed_user(&pool).await;
        let id = raw_tmi(&pool, &user).await;
        sqlx::query(
            "update tmu.tmis set stop_time = now() - interval '1 hour', start_time = now() - interval '3 hours' \
             where id = $1",
        )
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
        publish_then_expire(&pool, "tmu.tmis", &id).await;
        assert!(listed_tmi_ids(&pool).await.contains(&id));

        assert!(delete_tmi(&pool, &id).await.unwrap());

        assert!(!listed_tmi_ids(&pool).await.contains(&id));
        assert!(row_exists(&pool, "tmu.tmis", &id).await);
        assert_eq!(
            get_tmi(&pool, &id).await.unwrap().unwrap().status,
            "expired",
            "history keeps its real status, not cancelled"
        );
        let live_then = list_tmis_at(&pool, Utc::now() - chrono::Duration::minutes(90))
            .await
            .unwrap();
        assert!(
            live_then.iter().any(|t| t.id == id),
            "replay still shows it while it was live"
        );
    }

    #[sqlx::test]
    async fn deleting_an_active_published_tmi_cancels_it_and_removes_it_from_the_list(
        pool: PgPool,
    ) {
        let user = seed_user(&pool).await;
        let id = raw_tmi(&pool, &user).await;
        sqlx::query("update tmu.tmis set status = 'published', published_at = now() where id = $1")
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();

        assert!(delete_tmi(&pool, &id).await.unwrap());

        let row = get_tmi(&pool, &id).await.unwrap().unwrap();
        assert_eq!(row.status, "cancelled");
        assert!(!listed_tmi_ids(&pool).await.contains(&id));
        // No stop_time, so only the ended_at stamp stops replay showing it as live forever.
        let live_after = list_tmis_at(&pool, Utc::now() + chrono::Duration::seconds(1))
            .await
            .unwrap();
        assert!(
            !live_after.iter().any(|t| t.id == id),
            "replay ends it at the delete"
        );
    }

    #[sqlx::test]
    async fn deleting_a_draft_tmi_still_hard_deletes(pool: PgPool) {
        let user = seed_user(&pool).await;
        let id = raw_tmi(&pool, &user).await;

        assert!(delete_tmi(&pool, &id).await.unwrap());

        assert!(!row_exists(&pool, "tmu.tmis", &id).await);
    }

    #[sqlx::test]
    async fn deleting_an_unknown_id_reports_not_found(pool: PgPool) {
        assert!(
            !delete_tmi(&pool, "00000000-0000-0000-0000-000000000000")
                .await
                .unwrap()
        );
    }

    #[sqlx::test]
    async fn deleting_an_expired_published_ground_stop_removes_it_from_the_list(pool: PgPool) {
        let id = sqlx::query_scalar::<_, String>(
            "insert into tmu.ground_stops (airport) values ('KEWR') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        publish_then_expire(&pool, "tmu.ground_stops", &id).await;

        assert!(delete_ground_stop(&pool, &id).await.unwrap());

        let listed = list_ground_stops(&pool).await.unwrap();
        assert!(!listed.iter().any(|g| g.id == id));
        assert!(row_exists(&pool, "tmu.ground_stops", &id).await);
    }

    #[sqlx::test]
    async fn deleting_an_expired_published_gdp_removes_it_from_the_list(pool: PgPool) {
        let id = sqlx::query_scalar::<_, String>(
            "insert into tmu.gdp (airport, aar, start_time, end_time) \
             values ('KATL', 30, '1800', '2000') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        publish_then_expire(&pool, "tmu.gdp", &id).await;

        assert!(crate::repos::gdp::delete_gdp(&pool, &id).await.unwrap());

        let listed = crate::repos::gdp::list_gdps(&pool).await.unwrap();
        assert!(!listed.iter().any(|g| g.id == id));
        assert!(row_exists(&pool, "tmu.gdp", &id).await);
    }

    // --- advisory expiry (#537) -------------------------------------------------------------------

    /// The grace period `run_cleanup` applies after `valid_to`, as minutes. Named so the fixtures
    /// below can straddle it without being *derived* from it: a fixture computed off this constant
    /// would move with it and pass for any value, which is exactly how a widened constant gets
    /// shipped unnoticed.
    const GRACE_MIN: i64 = 30;

    /// Create an advisory with an explicit window, `age_min` minutes past its `valid_to`.
    async fn advisory_ending_min_ago(pool: &PgPool, age_min: i64) -> String {
        let user = seed_user(pool).await;
        let valid_to = Utc::now() - chrono::Duration::minutes(age_min);
        create_advisory(
            pool,
            &CreateAdvisoryRequest {
                facility: "DCC".to_string(),
                kind: "reroute".to_string(),
                body: "vATCSCC ADVZY".to_string(),
                structured: None,
                decoded: None,
                valid_from: Some(valid_to - chrono::Duration::hours(1)),
                valid_to: Some(valid_to),
            },
            &user,
        )
        .await
        .unwrap()
    }

    async fn status_of(pool: &PgPool, id: &str) -> String {
        sqlx::query_scalar::<_, String>("select status from tmu.advisories where id = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// Absolute ages straddling the documented grace period, not ages derived from `GRACE_MIN` — plus
    /// a guard that fails loudly if the constant moves out from between them.
    const INSIDE_GRACE_MIN: i64 = 20;
    const PAST_GRACE_MIN: i64 = 40;

    // Checked at compile time rather than in a test: all three are constants, so this can fail the
    // build instead of waiting for someone to run the suite — and clippy correctly refuses a runtime
    // assertion over a constant comparison.
    const _: () = assert!(
        INSIDE_GRACE_MIN < GRACE_MIN && GRACE_MIN < PAST_GRACE_MIN,
        "the advisory grace period is no longer between the two expiry fixtures — update \
         INSIDE_GRACE_MIN and PAST_GRACE_MIN, and the SQL interval in run_cleanup that they test"
    );

    /// AC3/AC7: past the window plus the grace period, the advisory is cancelled.
    #[sqlx::test]
    async fn cleanup_cancels_an_advisory_past_its_window(pool: PgPool) {
        let id = advisory_ending_min_ago(&pool, PAST_GRACE_MIN).await;

        let stats = run_cleanup(&pool).await.unwrap();

        assert_eq!(status_of(&pool, &id).await, "cancelled");
        assert_eq!(stats.advisories_cancelled, 1, "and it is counted");
    }

    /// AC7's other half: inside the grace period it is left alone. Without this, widening the
    /// interval to zero would still look correct.
    #[sqlx::test]
    async fn cleanup_spares_an_advisory_inside_the_grace_period(pool: PgPool) {
        let id = advisory_ending_min_ago(&pool, INSIDE_GRACE_MIN).await;

        let stats = run_cleanup(&pool).await.unwrap();

        assert_eq!(status_of(&pool, &id).await, "draft", "still within grace");
        assert_eq!(stats.advisories_cancelled, 0);
    }

    /// A still-current advisory must not be touched at all — the case a bad `>`/`<` would break.
    #[sqlx::test]
    async fn cleanup_spares_an_advisory_whose_window_is_open(pool: PgPool) {
        let id = advisory_ending_min_ago(&pool, -60).await; // ends in an hour

        run_cleanup(&pool).await.unwrap();

        assert_eq!(status_of(&pool, &id).await, "draft");
    }

    /// NULL `valid_to` means "no window", which must not read as "infinitely overdue". Every advisory
    /// written before #537 is in this state, so getting it wrong would cancel the lot on first boot.
    #[sqlx::test]
    async fn cleanup_never_touches_an_advisory_with_no_window(pool: PgPool) {
        let adv = draft(&pool, "DCC").await;
        assert!(adv.valid_to.is_none(), "the fixture has no window");

        let stats = run_cleanup(&pool).await.unwrap();

        assert_eq!(status_of(&pool, &adv.id).await, "draft");
        assert_eq!(stats.advisories_cancelled, 0);
    }

    /// A cancelled advisory must not be re-counted on every later pass — the count drives a log line,
    /// and a pass that claims work it did not do is worse than a quiet one.
    #[sqlx::test]
    async fn cleanup_does_not_recount_an_already_cancelled_advisory(pool: PgPool) {
        advisory_ending_min_ago(&pool, PAST_GRACE_MIN).await;
        assert_eq!(run_cleanup(&pool).await.unwrap().advisories_cancelled, 1);

        let second = run_cleanup(&pool).await.unwrap();

        assert_eq!(second.advisories_cancelled, 0, "nothing left to cancel");
    }

    /// Create a published ground stop and the advisory generated from it, linked as the real publish
    /// path links them.
    async fn published_gs_with_advisory(pool: &PgPool) -> (String, String) {
        let user = seed_user(pool).await;
        let gs_id = sqlx::query_scalar::<_, String>(
            "insert into tmu.ground_stops (airport, scope, status, published_at, created_by) \
             values ('KDCA', 'ALL', 'published', now(), $1) returning id",
        )
        .bind(&user)
        .fetch_one(pool)
        .await
        .unwrap();

        let mut tx = pool.begin().await.unwrap();
        let adv_id = create_advisory_tx(
            &mut tx,
            &CreateAdvisoryRequest {
                facility: "DCC".to_string(),
                kind: "reroute".to_string(),
                body: "vATCSCC ADVZY".to_string(),
                structured: None,
                decoded: None,
                // A generated advisory carries no window of its own — its life is the program's.
                valid_from: None,
                valid_to: None,
            },
            &user,
            Some(AdvisoryProgram::GroundStop(&gs_id)),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        (gs_id, adv_id)
    }

    /// The pre-existing bug this change folds in: `cancel_program_advisory_tx` only ever ran on a
    /// *user* revising or cancelling a program, so a ground stop that simply timed out left its
    /// advisory `published` forever.
    #[sqlx::test]
    async fn cleanup_cancels_the_advisory_of_an_expired_ground_stop(pool: PgPool) {
        let (gs_id, adv_id) = published_gs_with_advisory(&pool).await;
        sqlx::query("update tmu.ground_stops set status = 'expired' where id = $1")
            .bind(&gs_id)
            .execute(&pool)
            .await
            .unwrap();

        run_cleanup(&pool).await.unwrap();

        assert_eq!(status_of(&pool, &adv_id).await, "cancelled");
        // And the program row is still there, which is what makes the link usable: `run_cleanup`'s
        // deletes require `published_at is null`, and this one published.
        let gs_left: i64 =
            sqlx::query_scalar("select count(*) from tmu.ground_stops where id = $1")
                .bind(&gs_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            gs_left, 1,
            "a published program survives the cleanup deletes"
        );
    }

    /// Publish `adv_id` and record its `adv_publish` post the way `handlers::tmu::publish_advisory`
    /// does. The outbound job is the only record of whether — and where — an advisory was posted.
    async fn post_advisory(pool: &PgPool, adv_id: &str, channel: &str) {
        let user = crate::scope_test_support::seed_user(pool).await;
        let mut tx = pool.begin().await.unwrap();
        assert!(publish_advisory(&mut tx, adv_id, &user).await.unwrap());
        let adv = get_advisory_tx(&mut tx, adv_id).await.unwrap().unwrap();
        crate::repos::integration::enqueue_job(
            &mut tx,
            "adv_publish",
            &crate::advisory::publish_job_payload(channel, &adv),
            Some("advisory"),
            Some(adv_id),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
    }

    /// The channels of every `adv_cancel` correction enqueued for `adv_id`.
    async fn cancel_jobs(pool: &PgPool, adv_id: &str) -> Vec<String> {
        sqlx::query_scalar(
            "select coalesce(payload->>'channel_id', '') from integration.outbound_jobs \
             where job_type = 'adv_cancel' and subject_type = 'advisory' and subject_id = $1",
        )
        .bind(adv_id)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    /// A posted advisory the cleanup cancels is withdrawn from Discord, in the channel it was posted to
    /// (#537 review). Before, the status changed and the post stood indefinitely, asserting an advisory
    /// that had lapsed — the outcome cancellation was chosen over a soft-dismiss to avoid.
    #[sqlx::test]
    async fn cleanup_withdraws_a_posted_advisory_from_discord(pool: PgPool) {
        let id = advisory_ending_min_ago(&pool, PAST_GRACE_MIN).await;
        post_advisory(&pool, &id, "chan-posted").await;

        run_cleanup(&pool).await.unwrap();

        assert_eq!(status_of(&pool, &id).await, "cancelled");
        assert_eq!(
            cancel_jobs(&pool, &id).await,
            vec!["chan-posted".to_string()],
            "exactly one correction, beside the post it corrects"
        );

        // The next pass finds nothing left to cancel, so it posts nothing again.
        run_cleanup(&pool).await.unwrap();
        assert_eq!(cancel_jobs(&pool, &id).await.len(), 1);
    }

    /// Nothing was posted, so there is nothing to withdraw — for a draft, and for a published advisory
    /// whose post was never enqueued (Discord unconfigured). A correction to a post that does not
    /// exist would be noise in the channel.
    #[sqlx::test]
    async fn cleanup_posts_nothing_for_an_advisory_that_was_never_posted(pool: PgPool) {
        let draft = advisory_ending_min_ago(&pool, PAST_GRACE_MIN).await;
        let unposted = advisory_ending_min_ago(&pool, PAST_GRACE_MIN).await;
        let user = crate::scope_test_support::seed_user(&pool).await;
        let mut tx = pool.begin().await.unwrap();
        assert!(publish_advisory(&mut tx, &unposted, &user).await.unwrap());
        tx.commit().await.unwrap();

        run_cleanup(&pool).await.unwrap();

        for id in [&draft, &unposted] {
            assert_eq!(status_of(&pool, id).await, "cancelled");
            assert!(
                cancel_jobs(&pool, id).await.is_empty(),
                "no post, so no correction"
            );
        }
    }

    /// The program arm withdraws too: a ground stop that simply timed out has its posted advisory
    /// corrected, not just its row.
    #[sqlx::test]
    async fn cleanup_withdraws_the_posted_advisory_of_an_expired_ground_stop(pool: PgPool) {
        let (gs_id, adv_id) = published_gs_with_advisory(&pool).await;
        post_advisory(&pool, &adv_id, "chan-gs").await;
        sqlx::query("update tmu.ground_stops set status = 'expired' where id = $1")
            .bind(&gs_id)
            .execute(&pool)
            .await
            .unwrap();

        run_cleanup(&pool).await.unwrap();

        assert_eq!(status_of(&pool, &adv_id).await, "cancelled");
        assert_eq!(
            cancel_jobs(&pool, &adv_id).await,
            vec!["chan-gs".to_string()]
        );
    }

    /// While the program is live, its advisory must stay live — otherwise the pass would cancel every
    /// generated advisory on the first tick after it was issued.
    #[sqlx::test]
    async fn cleanup_spares_the_advisory_of_a_live_ground_stop(pool: PgPool) {
        let (_gs_id, adv_id) = published_gs_with_advisory(&pool).await;

        run_cleanup(&pool).await.unwrap();

        assert_eq!(status_of(&pool, &adv_id).await, "draft");
    }

    /// A hand-authored advisory has no program link, so the program arm must not reach it. Without
    /// this, a `left join` written instead of `exists` would cancel everything.
    #[sqlx::test]
    async fn the_program_arm_ignores_a_hand_authored_advisory(pool: PgPool) {
        let (gs_id, _) = published_gs_with_advisory(&pool).await;
        let unlinked = draft(&pool, "DCC").await;
        sqlx::query("update tmu.ground_stops set status = 'expired' where id = $1")
            .bind(&gs_id)
            .execute(&pool)
            .await
            .unwrap();

        run_cleanup(&pool).await.unwrap();

        assert_eq!(
            status_of(&pool, &unlinked.id).await,
            "draft",
            "no gdp_id and no ground_stop_id means no program expiry applies"
        );
    }
}
