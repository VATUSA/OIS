//! Organization data: facilities (ARTCCs).

use sqlx::PgPool;

use crate::{errors::ApiError, models::FacilityBody};

/// Active facilities, ordered by code.
pub async fn list_facilities(pool: &PgPool) -> Result<Vec<FacilityBody>, ApiError> {
    sqlx::query_as::<_, FacilityBody>(
        "select id, name, region, active from org.facilities where active order by id",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// One facility by code (active or not).
pub async fn find_facility(pool: &PgPool, id: &str) -> Result<Option<FacilityBody>, ApiError> {
    sqlx::query_as::<_, FacilityBody>(
        "select id, name, region, active from org.facilities where id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}
