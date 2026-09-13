//! Editable airport surface geometry: gates/parking positions (points), ramp/apron areas
//! (polygons), taxiways (lines) — see migration 0067. Writes are facility-scoped in the handler;
//! the repo is unscoped. Explicit per-type functions, mirroring `airport_configs`/
//! `facility_documents` — no generic CRUD abstraction over the three geometry kinds.

use sqlx::PgPool;

use crate::{
    errors::ApiError,
    models::{
        AirportGateBody, AirportRampAreaBody, AirportTaxiwayBody, UpsertAirportGateRequest,
        UpsertAirportRampAreaRequest, UpsertAirportTaxiwayRequest,
    },
};

// ---- gates ----------------------------------------------------------------

const GATE_SELECT: &str =
    "select id, icao, name, lat, lon, source, updated_at from flow.airport_gate";

pub async fn list_gates(pool: &PgPool, icao: &str) -> Result<Vec<AirportGateBody>, ApiError> {
    sqlx::query_as::<_, AirportGateBody>(&format!("{GATE_SELECT} where icao = $1 order by name"))
        .bind(icao)
        .fetch_all(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

pub async fn get_gate(pool: &PgPool, id: &str) -> Result<Option<AirportGateBody>, ApiError> {
    sqlx::query_as::<_, AirportGateBody>(&format!("{GATE_SELECT} where id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

pub async fn create_gate(
    pool: &PgPool,
    icao: &str,
    req: &UpsertAirportGateRequest,
    actor: &str,
) -> Result<AirportGateBody, ApiError> {
    let id: String = sqlx::query_scalar(
        "insert into flow.airport_gate (icao, name, lat, lon, source, updated_by) \
         values ($1, $2, $3, $4, 'manual', $5) returning id",
    )
    .bind(icao)
    .bind(&req.name)
    .bind(req.lat)
    .bind(req.lon)
    .bind(actor)
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    get_gate(pool, &id).await?.ok_or(ApiError::Internal)
}

pub async fn update_gate(
    pool: &PgPool,
    id: &str,
    icao: &str,
    req: &UpsertAirportGateRequest,
    actor: &str,
) -> Result<Option<AirportGateBody>, ApiError> {
    let r = sqlx::query(
        "update flow.airport_gate set name = $3, lat = $4, lon = $5, updated_by = $6 \
         where id = $1 and icao = $2",
    )
    .bind(id)
    .bind(icao)
    .bind(&req.name)
    .bind(req.lat)
    .bind(req.lon)
    .bind(actor)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    if r.rows_affected() == 0 {
        return Ok(None);
    }
    get_gate(pool, id).await
}

pub async fn delete_gate(pool: &PgPool, id: &str, icao: &str) -> Result<bool, ApiError> {
    let r = sqlx::query("delete from flow.airport_gate where id = $1 and icao = $2")
        .bind(id)
        .bind(icao)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(r.rows_affected() > 0)
}

// ---- ramp / apron areas -----------------------------------------------------

const RAMP_AREA_SELECT: &str =
    "select id, icao, name, kind, rings, source, updated_at from flow.airport_ramp_area";

pub async fn list_ramp_areas(
    pool: &PgPool,
    icao: &str,
) -> Result<Vec<AirportRampAreaBody>, ApiError> {
    sqlx::query_as::<_, AirportRampAreaBody>(&format!(
        "{RAMP_AREA_SELECT} where icao = $1 order by name"
    ))
    .bind(icao)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn get_ramp_area(
    pool: &PgPool,
    id: &str,
) -> Result<Option<AirportRampAreaBody>, ApiError> {
    sqlx::query_as::<_, AirportRampAreaBody>(&format!("{RAMP_AREA_SELECT} where id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

pub async fn create_ramp_area(
    pool: &PgPool,
    icao: &str,
    req: &UpsertAirportRampAreaRequest,
    actor: &str,
) -> Result<AirportRampAreaBody, ApiError> {
    let rings = sqlx::types::Json(&req.rings);
    let id: String = sqlx::query_scalar(
        "insert into flow.airport_ramp_area (icao, name, kind, rings, source, updated_by) \
         values ($1, $2, $3, $4, 'manual', $5) returning id",
    )
    .bind(icao)
    .bind(&req.name)
    .bind(&req.kind)
    .bind(rings)
    .bind(actor)
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    get_ramp_area(pool, &id).await?.ok_or(ApiError::Internal)
}

pub async fn update_ramp_area(
    pool: &PgPool,
    id: &str,
    icao: &str,
    req: &UpsertAirportRampAreaRequest,
    actor: &str,
) -> Result<Option<AirportRampAreaBody>, ApiError> {
    let rings = sqlx::types::Json(&req.rings);
    let r = sqlx::query(
        "update flow.airport_ramp_area set name = $3, kind = $4, rings = $5, updated_by = $6 \
         where id = $1 and icao = $2",
    )
    .bind(id)
    .bind(icao)
    .bind(&req.name)
    .bind(&req.kind)
    .bind(rings)
    .bind(actor)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    if r.rows_affected() == 0 {
        return Ok(None);
    }
    get_ramp_area(pool, id).await
}

pub async fn delete_ramp_area(pool: &PgPool, id: &str, icao: &str) -> Result<bool, ApiError> {
    let r = sqlx::query("delete from flow.airport_ramp_area where id = $1 and icao = $2")
        .bind(id)
        .bind(icao)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(r.rows_affected() > 0)
}

// ---- taxiways ---------------------------------------------------------------

const TAXIWAY_SELECT: &str =
    "select id, icao, name, points, source, updated_at from flow.airport_taxiway";

pub async fn list_taxiways(pool: &PgPool, icao: &str) -> Result<Vec<AirportTaxiwayBody>, ApiError> {
    sqlx::query_as::<_, AirportTaxiwayBody>(&format!(
        "{TAXIWAY_SELECT} where icao = $1 order by name"
    ))
    .bind(icao)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn get_taxiway(pool: &PgPool, id: &str) -> Result<Option<AirportTaxiwayBody>, ApiError> {
    sqlx::query_as::<_, AirportTaxiwayBody>(&format!("{TAXIWAY_SELECT} where id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

pub async fn create_taxiway(
    pool: &PgPool,
    icao: &str,
    req: &UpsertAirportTaxiwayRequest,
    actor: &str,
) -> Result<AirportTaxiwayBody, ApiError> {
    let points = sqlx::types::Json(&req.points);
    let id: String = sqlx::query_scalar(
        "insert into flow.airport_taxiway (icao, name, points, source, updated_by) \
         values ($1, $2, $3, 'manual', $4) returning id",
    )
    .bind(icao)
    .bind(&req.name)
    .bind(points)
    .bind(actor)
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    get_taxiway(pool, &id).await?.ok_or(ApiError::Internal)
}

pub async fn update_taxiway(
    pool: &PgPool,
    id: &str,
    icao: &str,
    req: &UpsertAirportTaxiwayRequest,
    actor: &str,
) -> Result<Option<AirportTaxiwayBody>, ApiError> {
    let points = sqlx::types::Json(&req.points);
    let r = sqlx::query(
        "update flow.airport_taxiway set name = $3, points = $4, updated_by = $5 \
         where id = $1 and icao = $2",
    )
    .bind(id)
    .bind(icao)
    .bind(&req.name)
    .bind(points)
    .bind(actor)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    if r.rows_affected() == 0 {
        return Ok(None);
    }
    get_taxiway(pool, id).await
}

pub async fn delete_taxiway(pool: &PgPool, id: &str, icao: &str) -> Result<bool, ApiError> {
    let r = sqlx::query("delete from flow.airport_taxiway where id = $1 and icao = $2")
        .bind(id)
        .bind(icao)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(r.rows_affected() > 0)
}
