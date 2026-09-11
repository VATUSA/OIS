//! Reusable per-airport runway configurations (see migration 0036). Named configs with a
//! favored-wind rule + AAR/ADR, used to predict an event's rate from the forecast wind. Writes are
//! facility-scoped in the handler; the repo is unscoped.

use sqlx::PgPool;

use crate::{
    errors::ApiError,
    models::{AirportConfigBody, UpsertAirportConfigRequest},
};

const CONFIG_SELECT: &str = "select c.id, c.icao, c.name, c.aar, c.adr, c.landing_runways, \
    c.wind_from_deg, c.wind_to_deg, c.calm_default, c.artcc, c.updated_at, \
    u.display_name as updated_by \
    from flow.airport_config c left join identity.users u on u.id = c.updated_by";

pub async fn list_by_icao(pool: &PgPool, icao: &str) -> Result<Vec<AirportConfigBody>, ApiError> {
    sqlx::query_as::<_, AirportConfigBody>(&format!(
        "{CONFIG_SELECT} where c.icao = $1 order by c.calm_default desc, c.name"
    ))
    .bind(icao)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Every airport's configs, optionally scoped to one owning ARTCC — ordered by airport for the
/// all-airports list view.
pub async fn list_all(
    pool: &PgPool,
    artcc: Option<&str>,
) -> Result<Vec<AirportConfigBody>, ApiError> {
    sqlx::query_as::<_, AirportConfigBody>(&format!(
        "{CONFIG_SELECT} where ($1::text is null or c.artcc = $1) \
         order by c.icao, c.calm_default desc, c.name"
    ))
    .bind(artcc)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn get(pool: &PgPool, id: &str) -> Result<Option<AirportConfigBody>, ApiError> {
    sqlx::query_as::<_, AirportConfigBody>(&format!("{CONFIG_SELECT} where c.id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// Only one calm-default config per airport — clear any existing one (optionally excluding `keep`).
async fn clear_calm(pool: &PgPool, icao: &str, keep: Option<&str>) -> Result<(), ApiError> {
    sqlx::query(
        "update flow.airport_config set calm_default = false \
         where icao = $1 and calm_default and ($2::text is null or id <> $2)",
    )
    .bind(icao)
    .bind(keep)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(())
}

pub async fn create(
    pool: &PgPool,
    icao: &str,
    req: &UpsertAirportConfigRequest,
    artcc: &str,
    actor: &str,
) -> Result<AirportConfigBody, ApiError> {
    if req.calm_default {
        clear_calm(pool, icao, None).await?;
    }
    let id: String = sqlx::query_scalar(
        "insert into flow.airport_config \
             (icao, name, aar, adr, landing_runways, wind_from_deg, wind_to_deg, calm_default, artcc, updated_by) \
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) returning id",
    )
    .bind(icao)
    .bind(&req.name)
    .bind(req.aar)
    .bind(req.adr)
    .bind(&req.landing_runways)
    .bind(req.wind_from_deg)
    .bind(req.wind_to_deg)
    .bind(req.calm_default)
    .bind(artcc)
    .bind(actor)
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    get(pool, &id).await?.ok_or(ApiError::Internal)
}

pub async fn update(
    pool: &PgPool,
    id: &str,
    icao: &str,
    req: &UpsertAirportConfigRequest,
    actor: &str,
) -> Result<Option<AirportConfigBody>, ApiError> {
    if req.calm_default {
        clear_calm(pool, icao, Some(id)).await?;
    }
    let r = sqlx::query(
        "update flow.airport_config set \
             name = $3, aar = $4, adr = $5, landing_runways = $6, \
             wind_from_deg = $7, wind_to_deg = $8, calm_default = $9, updated_by = $10 \
         where id = $1 and icao = $2",
    )
    .bind(id)
    .bind(icao)
    .bind(&req.name)
    .bind(req.aar)
    .bind(req.adr)
    .bind(&req.landing_runways)
    .bind(req.wind_from_deg)
    .bind(req.wind_to_deg)
    .bind(req.calm_default)
    .bind(actor)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    if r.rows_affected() == 0 {
        return Ok(None);
    }
    get(pool, id).await
}

pub async fn delete(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    let r = sqlx::query("delete from flow.airport_config where id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(r.rows_affected() > 0)
}
