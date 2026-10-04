//! Monitor Alert Parameter overrides (#598, migration 0116). `load_all` builds the
//! [`SectorMaps`] cached in `AppState`; `upsert` is the only write (`handlers::monitor`).

use sqlx::PgPool;

use crate::{errors::ApiError, feed::sectors::SectorMaps};

/// Every stored override.
pub async fn load_all(pool: &PgPool) -> Result<SectorMaps, ApiError> {
    let rows = sqlx::query_as::<_, (String, String, i32)>(
        "select artcc, sector_id, map from flow.sector_map",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(rows
        .into_iter()
        .map(|(artcc, sector_id, map)| ((artcc, sector_id), map))
        .collect())
}

/// One sector's stored override, if any.
pub async fn get(pool: &PgPool, artcc: &str, sector_id: &str) -> Result<Option<i32>, ApiError> {
    sqlx::query_scalar("select map from flow.sector_map where artcc = $1 and sector_id = $2")
        .bind(artcc)
        .bind(sector_id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// Remove a sector's override, so it reads the default again (#706).
pub async fn delete(pool: &PgPool, artcc: &str, sector_id: &str) -> Result<(), ApiError> {
    sqlx::query("delete from flow.sector_map where artcc = $1 and sector_id = $2")
        .bind(artcc)
        .bind(sector_id)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(())
}

/// Set one sector's MAP. `map` must be positive (the table's check refuses anything else).
pub async fn upsert(
    pool: &PgPool,
    artcc: &str,
    sector_id: &str,
    map: i32,
    updated_by: Option<&str>,
) -> Result<(), ApiError> {
    sqlx::query(
        "insert into flow.sector_map (artcc, sector_id, map, updated_by) values ($1, $2, $3, $4) \
         on conflict (artcc, sector_id) \
         do update set map = excluded.map, updated_by = excluded.updated_by, updated_at = now()",
    )
    .bind(artcc)
    .bind(sector_id)
    .bind(map)
    .bind(updated_by)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(())
}
