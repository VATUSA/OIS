//! Per-facility "facility map" color-rule configuration (see migration 0043). One row per ARTCC: an
//! ordered jsonb `rules` blob + a default color. Facility-scoped write authorization is enforced in
//! the handler; the repo is unscoped.

use sqlx::{PgPool, types::Json};

use crate::{
    errors::ApiError,
    models::{ColorRule, UpsertFacilityMapConfigRequest},
};

/// The stored config for a facility, or `None` if it's never been configured.
pub async fn get(
    pool: &PgPool,
    facility_id: &str,
) -> Result<Option<(Vec<ColorRule>, String)>, ApiError> {
    let row = sqlx::query_as::<_, (Json<Vec<ColorRule>>, String)>(
        "select rules, default_color from flow.facility_map_config where facility_id = $1",
    )
    .bind(facility_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(row.map(|(rules, default_color)| (rules.0, default_color)))
}

/// Insert or replace a facility's color rules.
pub async fn upsert(
    pool: &PgPool,
    facility_id: &str,
    req: &UpsertFacilityMapConfigRequest,
    actor: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "insert into flow.facility_map_config (facility_id, rules, default_color, updated_by) \
         values ($1, $2, $3, $4) \
         on conflict (facility_id) do update set \
             rules = excluded.rules, default_color = excluded.default_color, \
             updated_by = excluded.updated_by",
    )
    .bind(facility_id)
    .bind(Json(&req.rules))
    .bind(&req.default_color)
    .bind(actor)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(())
}
