//! Flow Constrained Area (FCA) storage. Shared, server-side — one FCA set for everyone.

use crate::auth::principal::Attribution;
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::errors::ApiError;
use crate::models::{FcaBody, UpsertFcaRequest, UpsertRouteRequest};

const FCA_SELECT: &str = "select f.id, f.name, f.color, f.artcc, f.points, f.dests, \
    f.origins, f.fixes, f.scope, f.min_fl, f.max_fl, f.dir, f.mode, f.rate, f.mit, \
    f.enabled, f.manual_order, f.manual_seq, f.updated_at, \
    coalesce(u.display_name, a.display_name) as updated_by, \
    f.event_id, f.event_status, f.auto_publish \
    from flow.fca f left join identity.users u on u.id = f.updated_by \
    left join access.actors a on a.id = f.updated_by_actor";

/// Event FCAs are hidden from every live map and the metering engine until they're `published`;
/// planned + archived ones are only ever seen in their event's builder ([`list_event_fcas`]).
const NOT_HIDDEN_EVENT: &str = "(f.event_id is null or f.event_status = 'published')";

pub async fn list_fcas(pool: &PgPool) -> Result<Vec<FcaBody>, ApiError> {
    sqlx::query_as::<_, FcaBody>(&format!(
        "{FCA_SELECT} where f.deleted_at is null and {NOT_HIDDEN_EVENT} order by f.name"
    ))
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// FCAs that existed and were enabled at instant `at` (for historical replay). Enable/disable isn't
/// historized, so the current `enabled` flag is used — an FCA toggled off since then is excluded.
pub async fn list_fcas_at(pool: &PgPool, at: DateTime<Utc>) -> Result<Vec<FcaBody>, ApiError> {
    sqlx::query_as::<_, FcaBody>(&format!(
        "{FCA_SELECT} where f.enabled and f.created_at <= $1 \
           and (f.deleted_at is null or f.deleted_at > $1) and {NOT_HIDDEN_EVENT} order by f.name"
    ))
    .bind(at)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Every FCA belonging to one event — planned, published, and archived — for the event manager's
/// builder. Unlike [`list_fcas`] this ignores the publish gate (that's the whole point of the builder).
pub async fn list_event_fcas(pool: &PgPool, event_id: i64) -> Result<Vec<FcaBody>, ApiError> {
    sqlx::query_as::<_, FcaBody>(&format!(
        "{FCA_SELECT} where f.event_id = $1 and f.deleted_at is null \
           order by f.event_status, f.name"
    ))
    .bind(event_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn get_fca(pool: &PgPool, id: &str) -> Result<Option<FcaBody>, ApiError> {
    sqlx::query_as::<_, FcaBody>(&format!("{FCA_SELECT} where f.id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// Bind every FCA column from a normalized request. Shared by insert + update.
fn bind_fca<'q>(
    q: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    req: &'q UpsertFcaRequest,
    by: &'q Attribution,
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    q.bind(req.name.trim())
        .bind(req.color.as_deref().unwrap_or("#f59e0b"))
        .bind(req.artcc.trim().to_ascii_uppercase())
        .bind(sqlx::types::Json(&req.points))
        .bind(&req.dests)
        .bind(&req.origins)
        .bind(&req.fixes)
        .bind(&req.scope)
        .bind(req.min_fl)
        .bind(req.max_fl)
        .bind(req.dir.as_deref().unwrap_or("any"))
        .bind(req.mode.as_deref().unwrap_or("rate"))
        .bind(req.rate.unwrap_or(30).clamp(0, 240))
        .bind(req.mit.unwrap_or(15).clamp(0, 200))
        .bind(req.enabled.unwrap_or(true))
        .bind(&by.user_id)
        .bind(&by.actor_id)
}

pub async fn create_fca(
    pool: &PgPool,
    req: &UpsertFcaRequest,
    by: &Attribution,
) -> Result<String, ApiError> {
    let q = sqlx::query(
        "insert into flow.fca
             (name, color, artcc, points, dests, origins, fixes, scope, min_fl, max_fl,
              dir, mode, rate, mit, enabled, updated_by, created_by,
              updated_by_actor, created_by_actor)
         values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$16,$17,$17)
         returning id",
    );
    // bind_fca sets $1..$17 (the 15 shared columns, then the user → `*_by` and the actor →
    // `*_by_actor`); the created_* columns reuse $16/$17 in the SQL, so no extra bind is needed.
    let row = bind_fca(q, req, by)
        .fetch_one(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    use sqlx::Row;
    row.try_get::<String, _>("id")
        .map_err(|_| ApiError::Internal)
}

/// Create an FCA owned by an event. Same columns as [`create_fca`], plus `event_id` and a starting
/// `event_status = 'planned'` — so it's invisible on live maps until published.
pub async fn create_event_fca(
    pool: &PgPool,
    event_id: i64,
    req: &UpsertFcaRequest,
    by: &Attribution,
) -> Result<String, ApiError> {
    let q = sqlx::query(
        "insert into flow.fca
             (name, color, artcc, points, dests, origins, fixes, scope, min_fl, max_fl,
              dir, mode, rate, mit, enabled, updated_by, created_by,
              updated_by_actor, created_by_actor, event_id, event_status)
         values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$16,$17,$17,$18,'planned')
         returning id",
    );
    let row = bind_fca(q, req, by)
        .bind(event_id)
        .fetch_one(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    use sqlx::Row;
    row.try_get::<String, _>("id")
        .map_err(|_| ApiError::Internal)
}

pub async fn update_fca(
    pool: &PgPool,
    id: &str,
    req: &UpsertFcaRequest,
    by: &Attribution,
) -> Result<bool, ApiError> {
    let q = sqlx::query(
        "update flow.fca set
             name = $1, color = $2, artcc = $3, points = $4, dests = $5, origins = $6,
             fixes = $7, scope = $8, min_fl = $9, max_fl = $10, dir = $11, mode = $12,
             rate = $13, mit = $14, enabled = $15, updated_by = $16, updated_by_actor = $17
         where id = $18",
    );
    let result = bind_fca(q, req, by)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

/// Publish an event FCA (planned → published) so it shows on live maps; stamps `published_at`.
/// Scoped to the event; false if it isn't there or isn't currently `planned`.
pub async fn mark_event_fca_published(
    pool: &PgPool,
    event_id: i64,
    fca_id: &str,
) -> Result<bool, ApiError> {
    let r = sqlx::query(
        "update flow.fca set event_status = 'published', published_at = now() \
         where id = $1 and event_id = $2 and event_status = 'planned'",
    )
    .bind(fca_id)
    .bind(event_id)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(r.rows_affected() > 0)
}

/// Archive an event FCA (planned/published → archived); stamps `archived_at`. Kept as event history,
/// hidden from every live map. Scoped to the event; false if it isn't there or is already archived.
pub async fn mark_event_fca_archived(
    pool: &PgPool,
    event_id: i64,
    fca_id: &str,
) -> Result<bool, ApiError> {
    let r = sqlx::query(
        "update flow.fca set event_status = 'archived', archived_at = now() \
         where id = $1 and event_id = $2 and event_status in ('planned', 'published')",
    )
    .bind(fca_id)
    .bind(event_id)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(r.rows_affected() > 0)
}

/// Set an event FCA's auto-publish flag. Scoped to the event.
pub async fn set_event_fca_auto(
    pool: &PgPool,
    event_id: i64,
    fca_id: &str,
    auto: bool,
) -> Result<bool, ApiError> {
    let r = sqlx::query("update flow.fca set auto_publish = $3 where id = $1 and event_id = $2")
        .bind(fca_id)
        .bind(event_id)
        .bind(auto)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(r.rows_affected() > 0)
}

/// One automatic-lifecycle pass (run periodically by the scheduler):
///   * publish planned + `auto_publish` FCAs once their event is within 30 min of starting (and hasn't
///     ended yet — so a missed window still recovers on the next tick);
///   * archive any still-live (planned/published) FCA whose event has ended.
///
/// Returns the number of rows changed, so the caller can skip the realtime nudge when nothing moved.
pub async fn run_event_fca_lifecycle(pool: &PgPool) -> Result<u64, ApiError> {
    let published = sqlx::query(
        "update flow.fca f set event_status = 'published', published_at = now() \
         from events.event e \
         where f.event_id = e.id and f.event_status = 'planned' and f.auto_publish \
           and now() >= e.start_time - interval '30 minutes' and now() < e.end_time \
           and f.deleted_at is null",
    )
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    let archived = sqlx::query(
        "update flow.fca f set event_status = 'archived', archived_at = now() \
         from events.event e \
         where f.event_id = e.id and f.event_status in ('planned', 'published') \
           and now() >= e.end_time and f.deleted_at is null",
    )
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(published.rows_affected() + archived.rows_affected())
}

pub async fn delete_fca(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    // Soft-delete so the historical dashboard can still show the FCA during the window it existed.
    let result =
        sqlx::query("update flow.fca set deleted_at = now() where id = $1 and deleted_at is null")
            .bind(id)
            .execute(pool)
            .await
            .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

// --- flow map routes (shared filed-route strings, resolved on read) ---

/// The stored fields of a route; the handler resolves `route` to a track for the API response.
#[derive(Debug, sqlx::FromRow)]
pub struct RouteRow {
    pub id: String,
    pub name: String,
    pub color: String,
    pub route: String,
    pub dep: String,
    pub arr: String,
    pub artcc: Option<String>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub updated_by: Option<String>,
}

const ROUTE_SELECT: &str = "select r.id, r.name, r.color, r.route, r.dep, r.arr, r.artcc, \
    r.updated_at, coalesce(u.display_name, a.display_name) as updated_by \
    from flow.route r left join identity.users u on u.id = r.updated_by \
    left join access.actors a on a.id = r.updated_by_actor";

/// All routes, or — when `artcc` is given — that ARTCC's routes plus the global (NULL) ones.
pub async fn list_routes(pool: &PgPool, artcc: Option<&str>) -> Result<Vec<RouteRow>, ApiError> {
    let sql = match artcc {
        Some(_) => format!("{ROUTE_SELECT} where r.artcc = $1 or r.artcc is null order by r.name"),
        None => format!("{ROUTE_SELECT} order by r.name"),
    };
    let mut q = sqlx::query_as::<_, RouteRow>(&sql);
    if let Some(a) = artcc {
        q = q.bind(a);
    }
    q.fetch_all(pool).await.map_err(|_| ApiError::Internal)
}

pub async fn get_route(pool: &PgPool, id: &str) -> Result<Option<RouteRow>, ApiError> {
    sqlx::query_as::<_, RouteRow>(&format!("{ROUTE_SELECT} where r.id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

pub async fn create_route(
    pool: &PgPool,
    req: &UpsertRouteRequest,
    by: &Attribution,
) -> Result<String, ApiError> {
    sqlx::query_scalar::<_, String>(
        "insert into flow.route (name, color, route, dep, arr, artcc, updated_by, created_by, \
             updated_by_actor, created_by_actor) \
         values ($1, $2, $3, $4, $5, $6, $7, $7, $8, $8) returning id",
    )
    .bind(req.name.trim())
    .bind(req.color.as_deref().unwrap_or("#38bdf8"))
    .bind(req.route.trim())
    .bind(req.dep.as_deref().unwrap_or("").trim().to_ascii_uppercase())
    .bind(req.arr.as_deref().unwrap_or("").trim().to_ascii_uppercase())
    .bind(norm_artcc(req.artcc.as_deref()))
    .bind(&by.user_id)
    .bind(&by.actor_id)
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn update_route(
    pool: &PgPool,
    id: &str,
    req: &UpsertRouteRequest,
    by: &Attribution,
) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "update flow.route set name = $1, color = $2, route = $3, dep = $4, arr = $5, \
         artcc = $6, updated_by = $7, updated_by_actor = $8 where id = $9",
    )
    .bind(req.name.trim())
    .bind(req.color.as_deref().unwrap_or("#38bdf8"))
    .bind(req.route.trim())
    .bind(req.dep.as_deref().unwrap_or("").trim().to_ascii_uppercase())
    .bind(req.arr.as_deref().unwrap_or("").trim().to_ascii_uppercase())
    .bind(norm_artcc(req.artcc.as_deref()))
    .bind(&by.user_id)
    .bind(&by.actor_id)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

/// Normalize an ARTCC id: trimmed + uppercased, or `None` for blank (a global route).
pub fn norm_artcc(raw: Option<&str>) -> Option<String> {
    raw.map(|a| a.trim().to_ascii_uppercase())
        .filter(|a| !a.is_empty())
}

pub async fn delete_route(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    let result = sqlx::query("delete from flow.route where id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

/// Set (or clear) an FCA's manual crossing order.
pub async fn set_manual_order(
    pool: &PgPool,
    id: &str,
    order: &[String],
    manual_seq: bool,
    by: &Attribution,
) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "update flow.fca set manual_order = $2, manual_seq = $3, updated_by = $4, \
             updated_by_actor = $5 where id = $1",
    )
    .bind(id)
    .bind(order)
    .bind(manual_seq)
    .bind(&by.user_id)
    .bind(&by.actor_id)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

// --- frozen CFR releases ---

/// A writer's precondition on a release (#585): `If-None-Match: *` is [`Expect::Absent`], and
/// `If-Match: N` is [`Expect::Version`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    Absent,
    Version(i64),
}

/// Who holds a release now, and at what version (#585). `machine` is the holding actor's id and name
/// when a service account or API key wrote it last; `None` means a person did, including a legacy row
/// that predates actor attribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseHolder {
    pub version: i64,
    pub machine: Option<(String, String)>,
}

const HOLDER_SELECT: &str = "select r.callsign, r.version, \
        case when a.actor_type in ('service_account', 'api_key') then a.id end, \
        case when a.actor_type in ('service_account', 'api_key') then a.display_name end \
     from flow.fca_release r left join access.actors a on a.id = r.updated_by_actor";

fn holder_of(version: i64, id: Option<String>, name: Option<String>) -> ReleaseHolder {
    ReleaseHolder {
        version,
        machine: id.map(|id| (id, name.unwrap_or_default())),
    }
}

/// The holder of one release, or `None` when the flight is not released.
pub async fn release_holder(
    pool: &PgPool,
    fca_id: &str,
    callsign: &str,
) -> Result<Option<ReleaseHolder>, ApiError> {
    let row = sqlx::query_as::<_, (String, i64, Option<String>, Option<String>)>(&format!(
        "{HOLDER_SELECT} where r.fca_id = $1 and r.callsign = $2"
    ))
    .bind(fca_id)
    .bind(callsign)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(row.map(|(_, v, id, name)| holder_of(v, id, name)))
}

/// Every release's holder in one FCA, by callsign — read alongside the metering input, never part of
/// it, so provenance and versions cannot change a computed time.
pub async fn release_holders(
    pool: &PgPool,
    fca_id: &str,
) -> Result<HashMap<String, ReleaseHolder>, ApiError> {
    let rows = sqlx::query_as::<_, (String, i64, Option<String>, Option<String>)>(&format!(
        "{HOLDER_SELECT} where r.fca_id = $1"
    ))
    .bind(fca_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(rows
        .into_iter()
        .map(|(cs, v, id, name)| (cs, holder_of(v, id, name)))
        .collect())
}

/// Frozen releases for an FCA as (callsign, cta_ms, edct_ms).
pub async fn list_releases(
    pool: &PgPool,
    fca_id: &str,
) -> Result<Vec<(String, i64, i64)>, ApiError> {
    sqlx::query_as::<_, (String, i64, i64)>(
        "select callsign, cta_ms, edct_ms from flow.fca_release where fca_id = $1",
    )
    .bind(fca_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Earliest frozen FCA release (EDCT, epoch-ms) per callsign for the given callsigns, across
/// *enabled* FCAs only. Lets the departure-field view surface FCA-issued release times, so an FCA's
/// RDY/RLSD flows to the airport departures list — not just the FCA page.
pub async fn releases_for_callsigns(
    pool: &PgPool,
    callsigns: &[String],
) -> Result<HashMap<String, i64>, ApiError> {
    if callsigns.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query_as::<_, (String, i64)>(
        "select r.callsign, min(r.edct_ms) as edct \
         from flow.fca_release r join flow.fca f on f.id = r.fca_id \
         where f.enabled and r.callsign = any($1) \
         group by r.callsign",
    )
    .bind(callsigns)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(rows.into_iter().collect())
}

/// Write a release, returning its new version — or `None` when `expect` did not hold, in which case
/// nothing was written (#585). The precondition is checked **in the write itself**, so there is no
/// window between reading the version and changing the row.
pub async fn upsert_release(
    pool: &PgPool,
    fca_id: &str,
    callsign: &str,
    cta_ms: i64,
    edct_ms: i64,
    by: &Attribution,
    expect: Option<Expect>,
) -> Result<Option<i64>, ApiError> {
    let sql = match expect {
        // Unconditional (a person, as before): create or replace.
        None => {
            "insert into flow.fca_release as r (fca_id, callsign, cta_ms, edct_ms, updated_by, updated_by_actor)
             values ($1, $2, $3, $4, $5, $6)
             on conflict (fca_id, callsign) do update set
                 cta_ms = excluded.cta_ms, edct_ms = excluded.edct_ms,
                 updated_by = excluded.updated_by, updated_by_actor = excluded.updated_by_actor,
                 version = r.version + 1
             returning r.version"
        }
        // Create only: an existing release makes this a no-op, so a retry cannot issue twice.
        Some(Expect::Absent) => {
            "insert into flow.fca_release (fca_id, callsign, cta_ms, edct_ms, updated_by, updated_by_actor)
             values ($1, $2, $3, $4, $5, $6)
             on conflict (fca_id, callsign) do nothing
             returning version"
        }
        // Replace only the version the writer last saw.
        Some(Expect::Version(_)) => {
            "update flow.fca_release set
                 cta_ms = $3, edct_ms = $4, updated_by = $5, updated_by_actor = $6,
                 version = version + 1
             where fca_id = $1 and callsign = $2 and version = $7
               and ($8::text is null or updated_by_actor = $8)
             returning version"
        }
    };
    let query = sqlx::query_scalar::<_, i64>(sql)
        .bind(fca_id)
        .bind(callsign)
        .bind(cta_ms)
        .bind(edct_ms)
        .bind(&by.user_id)
        .bind(&by.actor_id);
    // `$7` exists only in the conditional update; binding it elsewhere is a parameter-count error.
    // `$7`/`$8` exist only in the conditional update; binding them elsewhere is a parameter-count
    // error. `$8` is the machine that must already hold the row (#585 review).
    let query = match expect {
        Some(Expect::Version(v)) => query.bind(v).bind(by.machine_actor()),
        _ => query,
    };
    query
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// Exchange two releases' frozen times within one FCA (#514).
///
/// Returns whether the swap happened — `false` means at least one of the two callsigns holds no
/// release, and nothing was written.
///
/// # One statement, deliberately
///
/// `update … from` over the same table is atomic by itself and reads the **pre-statement snapshot**,
/// so each row receives the other's values. A read-then-write pair would need an explicit
/// transaction and could still interleave with a concurrent [`upsert_release`], leaving one row
/// holding the other's time and the other holding its own.
///
/// `rows_affected() == 2` is the success condition rather than a separate existence check: if either
/// callsign has no release the self-join matches nothing, zero rows change, and a partial swap is
/// impossible. Passing the same callsign twice matches one row, which is also not 2 — though the
/// handler rejects that earlier with a clearer error.
///
/// # Why this is not the reorder
///
/// `set_manual_order` is the other way to change who goes first, and it is defined to re-chain
/// everyone behind the moved aircraft (`feed::fca`'s manual branch). This touches two rows and
/// nothing else, which is what lets two flights trade slots without renumbering the field.
pub async fn swap_releases(
    pool: &PgPool,
    fca_id: &str,
    a: &str,
    b: &str,
    by: &Attribution,
) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "update flow.fca_release r \
            set cta_ms = o.cta_ms, edct_ms = o.edct_ms, updated_by = $4, updated_by_actor = $5, \
                version = r.version + 1 \
           from flow.fca_release o \
          where r.fca_id = $1 and o.fca_id = $1 \
            and ((r.callsign = $2 and o.callsign = $3) \
              or (r.callsign = $3 and o.callsign = $2)) \
            and ($6::text is null or (r.updated_by_actor = $6 and o.updated_by_actor = $6))",
    )
    .bind(fca_id)
    .bind(a)
    .bind(b)
    .bind(&by.user_id)
    .bind(&by.actor_id)
    // A machine may swap only two releases it holds itself, decided in the write (#585 review).
    .bind(by.machine_actor())
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() == 2)
}

/// Clear a release, only at `version` when given and, for a machine, only if it already holds it
/// (#585). The holder comes from `by` here, as in the update and swap writers, rather than from the
/// caller: no route can reach the race this clause closes, so a call site that dropped it would go
/// unnoticed. Returns whether a row was removed.
pub async fn delete_release(
    pool: &PgPool,
    fca_id: &str,
    callsign: &str,
    version: Option<i64>,
    by: &Attribution,
) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "delete from flow.fca_release \
         where fca_id = $1 and callsign = $2 and ($3::bigint is null or version = $3) \
           and ($4::text is null or updated_by_actor = $4)",
    )
    .bind(fca_id)
    .bind(callsign)
    .bind(version)
    .bind(by.machine_actor())
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}
