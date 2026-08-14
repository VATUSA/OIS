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

/// Insert or update the config for `icao`. Every field is preserved when its argument is
/// `None` (`coalesce` keeps the existing value), so a partial `PUT` that touches only, say,
/// the active ends leaves another controller's STAR rules and overrides intact.
#[allow(clippy::too_many_arguments)]
pub async fn upsert_config(
    pool: &PgPool,
    icao: &str,
    active_ends: Option<&[String]>,
    star_rules: Option<&HashMap<String, String>>,
    overrides: Option<&HashMap<String, String>>,
    window_min: Option<i32>,
    user_id: &str,
    custom_ends: Option<&Vec<crate::feed::runway::CustomEnd>>,
) -> Result<(), ApiError> {
    sqlx::query(
        "insert into flow.runway_config \
           (icao, active_ends, star_rules, overrides, window_min, updated_by, custom_ends) \
         values ($1, coalesce($2, '{}'::text[]), coalesce($3, '{}'::jsonb), \
                 coalesce($4, '{}'::jsonb), coalesce($5, 90), $6, coalesce($7, '[]'::jsonb)) \
         on conflict (icao) do update set \
           active_ends = coalesce($2, flow.runway_config.active_ends), \
           star_rules = coalesce($3, flow.runway_config.star_rules), \
           overrides = coalesce($4, flow.runway_config.overrides), \
           window_min = coalesce($5, flow.runway_config.window_min), \
           updated_by = $6, \
           custom_ends = coalesce($7, flow.runway_config.custom_ends)",
    )
    .bind(icao)
    .bind(active_ends)
    .bind(star_rules.map(Json))
    .bind(overrides.map(Json))
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
