//! TMU persistence — Traffic Management Initiatives (TMIs).

use sqlx::{PgPool, Postgres, Transaction};

use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::{
    errors::ApiError,
    models::{
        CreateGroundStopRequest, CreateTmiRequest, GateRule, GroundStopBody, IssuedCfrBody,
        ProgramBody, TmiBody, UpdateTmiRequest, UpsertProgramRequest,
    },
};

const SELECT: &str = "select t.id, t.requesting, t.providing, t.restriction, \
    t.start_time, t.stop_time, t.status, t.published_at, t.created_at, \
    u.display_name as author \
    from tmu.tmis t left join identity.users u on u.id = t.created_by";

pub async fn list_tmis(pool: &PgPool, status: Option<&str>) -> Result<Vec<TmiBody>, ApiError> {
    sqlx::query_as::<_, TmiBody>(&format!(
        "{SELECT} where ($1::text is null or t.status = $1) order by t.created_at desc"
    ))
    .bind(status)
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

pub async fn create_tmi(
    pool: &PgPool,
    req: &CreateTmiRequest,
    created_by: &str,
) -> Result<String, ApiError> {
    sqlx::query_scalar::<_, String>(
        "insert into tmu.tmis \
         (requesting, providing, restriction, start_time, stop_time, created_by) \
         values ($1, $2, $3, coalesce($4, now()), $5, $6) returning id",
    )
    .bind(&req.requesting)
    .bind(&req.providing)
    .bind(&req.restriction)
    .bind(req.start_time)
    .bind(req.stop_time)
    .bind(created_by)
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Updates the given fields (COALESCE — omitted fields are left unchanged). Returns
/// false if the TMI doesn't exist.
pub async fn update_tmi(pool: &PgPool, id: &str, req: &UpdateTmiRequest) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "update tmu.tmis set \
            requesting = coalesce($2, requesting), \
            providing = coalesce($3, providing), \
            restriction = coalesce($4, restriction), \
            start_time = coalesce($5, start_time), \
            stop_time = coalesce($6, stop_time) \
         where id = $1",
    )
    .bind(id)
    .bind(&req.requesting)
    .bind(&req.providing)
    .bind(&req.restriction)
    .bind(req.start_time)
    .bind(req.stop_time)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
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
pub async fn cancel_tmi(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "update tmu.tmis set status = 'cancelled', ended_at = coalesce(ended_at, now()) \
         where id = $1 and status in ('draft', 'published')",
    )
    .bind(id)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

pub async fn delete_tmi(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    delete_or_retain(pool, "tmu.tmis", id).await
}

/// Delete for the published-history entities (TMIs / ground stops / GDPs): a row that was NEVER
/// published (a draft dropped without going live) is hard-deleted and never appears in replay; a
/// row that was ever published is KEPT — cancelled (with `ended_at` stamped if still active) so the
/// historical dashboard can still show it during the window it was live. `table` is a trusted
/// internal literal, never user input. Returns whether a row with that id existed.
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
                            else ended_at end \
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
pub async fn upsert_program(
    pool: &PgPool,
    icao: &str,
    req: &UpsertProgramRequest,
    gates: &[GateRule],
    actor: &str,
) -> Result<(), ApiError> {
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
    .execute(pool)
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
    sqlx::query_as::<_, GroundStopBody>(&format!("{GS_SELECT} order by g.updated_at desc"))
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

pub async fn get_ground_stop(pool: &PgPool, id: &str) -> Result<Option<GroundStopBody>, ApiError> {
    sqlx::query_as::<_, GroundStopBody>(&format!("{GS_SELECT} where g.id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

pub async fn create_ground_stop(
    pool: &PgPool,
    req: &CreateGroundStopRequest,
    scope: &str,
    until: Option<&str>,
    actor: &str,
) -> Result<String, ApiError> {
    sqlx::query_scalar::<_, String>(
        "insert into tmu.ground_stops (airport, scope, until, created_by, updated_by) \
         values ($1, $2, $3, $4, $4) returning id",
    )
    .bind(&req.airport)
    .bind(scope)
    .bind(until)
    .bind(actor)
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Publishes a draft ground stop. Returns false if it isn't currently a draft.
pub async fn publish_ground_stop(
    pool: &PgPool,
    id: &str,
    published_by: &str,
) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "update tmu.ground_stops set status = 'published', published_by = $2, published_at = now() \
         where id = $1 and status = 'draft'",
    )
    .bind(id)
    .bind(published_by)
    .execute(pool)
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

pub async fn upsert_issued_cfr(
    pool: &PgPool,
    callsign: &str,
    airport: &str,
    wheels_up: DateTime<Utc>,
    actor: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "insert into tmu.issued_cfrs (callsign, airport, wheels_up, issued_by) \
         values ($1, $2, $3, $4) \
         on conflict (callsign) do update set \
            airport = excluded.airport, wheels_up = excluded.wheels_up, \
            issued_by = excluded.issued_by, issued_at = now()",
    )
    .bind(callsign)
    .bind(airport)
    .bind(wheels_up)
    .bind(actor)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(())
}

pub async fn get_issued_cfr(
    pool: &PgPool,
    callsign: &str,
) -> Result<Option<IssuedCfrBody>, ApiError> {
    sqlx::query_as::<_, IssuedCfrBody>(
        "select c.callsign, c.airport, c.wheels_up, u.display_name as issued_by, c.issued_at \
         from tmu.issued_cfrs c left join identity.users u on u.id = c.issued_by \
         where c.callsign = $1",
    )
    .bind(callsign)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn delete_issued_cfr(pool: &PgPool, callsign: &str) -> Result<bool, ApiError> {
    let result = sqlx::query("delete from tmu.issued_cfrs where callsign = $1")
        .bind(callsign)
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

    Ok(CleanupStats {
        expired: e_tmi.rows_affected() + e_gs.rows_affected() + e_gdp.rows_affected(),
        deleted: d_tmi.rows_affected()
            + d_gs.rows_affected()
            + d_pgm.rows_affected()
            + d_gdp.rows_affected(),
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
