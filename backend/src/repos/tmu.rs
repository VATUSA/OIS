//! TMU persistence — Traffic Management Initiatives (TMIs).

use sqlx::PgPool;

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
pub async fn publish_tmi(pool: &PgPool, id: &str, published_by: &str) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "update tmu.tmis set status = 'published', published_by = $2, published_at = now() \
         where id = $1 and status = 'draft'",
    )
    .bind(id)
    .bind(published_by)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

/// Cancels a draft or published TMI. Returns false if it's already terminal.
pub async fn cancel_tmi(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "update tmu.tmis set status = 'cancelled' \
         where id = $1 and status in ('draft', 'published')",
    )
    .bind(id)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

pub async fn delete_tmi(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    let result = sqlx::query("delete from tmu.tmis where id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

// --- rate programs ---

const PROGRAM_SELECT: &str = "select p.icao, p.aar, p.trail, p.mit, p.gates, \
    p.exclude_wake, p.exclude_types, p.jets_only, p.updated_at, \
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
         (icao, aar, trail, mit, gates, exclude_wake, exclude_types, jets_only, created_by, updated_by) \
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9) \
         on conflict (icao) do update set \
            aar = excluded.aar, trail = excluded.trail, mit = excluded.mit, \
            gates = excluded.gates, exclude_wake = excluded.exclude_wake, \
            exclude_types = excluded.exclude_types, jets_only = excluded.jets_only, \
            updated_by = excluded.updated_by",
    )
    .bind(icao)
    .bind(req.aar)
    .bind(req.trail)
    .bind(req.mit)
    .bind(sqlx::types::Json(gates))
    .bind(&req.exclude_wake)
    .bind(&req.exclude_types)
    .bind(req.jets_only)
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
        "update tmu.ground_stops set status = 'cancelled' \
         where id = $1 and status in ('draft', 'published')",
    )
    .bind(id)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

pub async fn delete_ground_stop(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    let result = sqlx::query("delete from tmu.ground_stops where id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
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
