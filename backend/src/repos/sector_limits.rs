//! Sector occupancy limit overrides (#722, migration 0127, `flow.sector_limit`). `load_all` builds the
//! [`SectorLimits`] cached in `AppState`; `handlers::sector_limits` is the only writer.

use sqlx::PgPool;

use crate::{errors::ApiError, feed::sector_limits::SectorLimits};

/// Every stored override.
pub async fn load_all(pool: &PgPool) -> Result<SectorLimits, ApiError> {
    let rows = sqlx::query_as::<_, (String, String, i32)>(
        "select artcc, sector_id, limit_value from flow.sector_limit",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(rows
        .into_iter()
        .map(|(artcc, sector_id, limit)| ((artcc, sector_id), limit))
        .collect())
}

/// One sector's stored override, if any.
pub async fn get(pool: &PgPool, artcc: &str, sector_id: &str) -> Result<Option<i32>, ApiError> {
    sqlx::query_scalar::<_, i32>(
        "select limit_value from flow.sector_limit where artcc = $1 and sector_id = $2",
    )
    .bind(artcc)
    .bind(sector_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Set one sector's override. `limit` must be positive (the table's check refuses anything else).
pub async fn upsert(
    pool: &PgPool,
    artcc: &str,
    sector_id: &str,
    limit: i32,
    updated_by: Option<&str>,
) -> Result<(), ApiError> {
    sqlx::query(
        "insert into flow.sector_limit (artcc, sector_id, limit_value, updated_by) \
         values ($1, $2, $3, $4) \
         on conflict (artcc, sector_id) do update set \
         limit_value = excluded.limit_value, updated_by = excluded.updated_by, updated_at = now()",
    )
    .bind(artcc)
    .bind(sector_id)
    .bind(limit)
    .bind(updated_by)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(())
}

/// Remove one sector's override, returning it to the default. Touches only that `(artcc, sector_id)`.
pub async fn delete(pool: &PgPool, artcc: &str, sector_id: &str) -> Result<(), ApiError> {
    sqlx::query("delete from flow.sector_limit where artcc = $1 and sector_id = $2")
        .bind(artcc)
        .bind(sector_id)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(())
}
