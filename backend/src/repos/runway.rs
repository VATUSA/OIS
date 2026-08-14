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
}

/// The stored config for `icao`, or None if the airport has never been configured.
pub async fn get_config(pool: &PgPool, icao: &str) -> Result<Option<RunwayConfigRow>, ApiError> {
    sqlx::query_as::<_, RunwayConfigRow>(
        "select active_ends, star_rules, overrides, window_min \
         from flow.runway_config where icao = $1",
    )
    .bind(icao)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Insert or replace the config for `icao`.
#[allow(clippy::too_many_arguments)]
pub async fn upsert_config(
    pool: &PgPool,
    icao: &str,
    active_ends: &[String],
    star_rules: &HashMap<String, String>,
    overrides: &HashMap<String, String>,
    window_min: i32,
    user_id: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "insert into flow.runway_config \
           (icao, active_ends, star_rules, overrides, window_min, updated_by) \
         values ($1, $2, $3, $4, $5, $6) \
         on conflict (icao) do update set \
           active_ends = $2, star_rules = $3, overrides = $4, window_min = $5, updated_by = $6",
    )
    .bind(icao)
    .bind(active_ends)
    .bind(Json(star_rules))
    .bind(Json(overrides))
    .bind(window_min)
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(())
}
