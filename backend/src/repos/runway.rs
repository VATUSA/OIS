//! Persistence for the Runway Balancer — shared per-airport runway config (active ends,
//! STAR→runway rules, aircraft overrides).

use std::collections::HashMap;

use sqlx::PgPool;
use sqlx::types::Json;

use crate::errors::ApiError;

/// Stored runway configuration for one airport.
#[derive(Debug, sqlx::FromRow)]
pub struct RunwayConfigRow {
    pub active_ends: Vec<String>,
    pub star_rules: Json<HashMap<String, String>>,
    pub overrides: Json<HashMap<String, String>>,
    pub window_min: i32,
    pub custom_ends: Json<Vec<crate::feed::runway::CustomEnd>>,
}

/// The stored config for `icao`, or None if the airport has never been configured.
pub async fn get_config(pool: &PgPool, icao: &str) -> Result<Option<RunwayConfigRow>, ApiError> {
    sqlx::query_as::<_, RunwayConfigRow>(
        "select active_ends, star_rules, overrides, window_min, custom_ends \
         from flow.runway_config where icao = $1",
    )
    .bind(icao)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Insert or replace the config for `icao`. `custom_ends` is preserved when `None`
/// (`coalesce` keeps the existing manual ends).
#[allow(clippy::too_many_arguments)]
pub async fn upsert_config(
    pool: &PgPool,
    icao: &str,
    active_ends: &[String],
    star_rules: &HashMap<String, String>,
    overrides: &HashMap<String, String>,
    window_min: i32,
    user_id: &str,
    custom_ends: Option<&Vec<crate::feed::runway::CustomEnd>>,
) -> Result<(), ApiError> {
    sqlx::query(
        "insert into flow.runway_config \
           (icao, active_ends, star_rules, overrides, window_min, updated_by, custom_ends) \
         values ($1, $2, $3, $4, $5, $6, coalesce($7, '[]'::jsonb)) \
         on conflict (icao) do update set \
           active_ends = $2, star_rules = $3, overrides = $4, window_min = $5, updated_by = $6, \
           custom_ends = coalesce($7, flow.runway_config.custom_ends)",
    )
    .bind(icao)
    .bind(active_ends)
    .bind(Json(star_rules))
    .bind(Json(overrides))
    .bind(window_min)
    .bind(user_id)
    .bind(custom_ends.map(Json))
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(())
}

/// The active-ends + STAR-rules payload of a named runway config.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct SavedPayload {
    #[serde(default)]
    pub active_ends: Vec<String>,
    #[serde(default)]
    pub star_rules: HashMap<String, String>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct SavedConfigRow {
    pub name: String,
    pub payload: Json<SavedPayload>,
}

/// All named configs saved for `icao`, alphabetically.
pub async fn list_saved(pool: &PgPool, icao: &str) -> Result<Vec<SavedConfigRow>, ApiError> {
    sqlx::query_as::<_, SavedConfigRow>(
        "select name, payload from flow.runway_saved_config where icao = $1 order by name",
    )
    .bind(icao)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Insert or replace a named config for `icao`.
pub async fn upsert_saved(
    pool: &PgPool,
    icao: &str,
    name: &str,
    payload: &SavedPayload,
    user_id: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "insert into flow.runway_saved_config (icao, name, payload, updated_by) \
         values ($1, $2, $3, $4) \
         on conflict (icao, name) do update set payload = $3, updated_by = $4",
    )
    .bind(icao)
    .bind(name)
    .bind(Json(payload))
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(())
}

/// Delete a named config; returns whether a row was removed.
pub async fn delete_saved(pool: &PgPool, icao: &str, name: &str) -> Result<bool, ApiError> {
    let r = sqlx::query("delete from flow.runway_saved_config where icao = $1 and name = $2")
        .bind(icao)
        .bind(name)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(r.rows_affected() > 0)
}
