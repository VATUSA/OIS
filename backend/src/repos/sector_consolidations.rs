//! Sector consolidations (#599, migration 0118). `load_all` builds the [`Consolidations`] cached in
//! `AppState`; `consolidate` and `release` are the only writes (`handlers::monitor`).

use sqlx::PgPool;

use crate::{errors::ApiError, feed::monitor::Consolidations};

/// Every stored consolidation.
pub async fn load_all(pool: &PgPool) -> Result<Consolidations, ApiError> {
    let rows = sqlx::query_as::<_, (String, String, String)>(
        "select artcc, sector_id, target_sector_id from flow.sector_consolidation",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(rows
        .into_iter()
        .map(|(artcc, source, target)| ((artcc, source), target))
        .collect())
}

/// Work `source` at `target`'s position, keeping the arrangement flat, all in one transaction — a
/// refused or failed save leaves the previous arrangement exactly as it was.
///
/// - A target that is itself worked elsewhere resolves to where it is worked.
/// - If that resolves back to `source` (a at b, then b at a) it is a loop: `Conflict`, nothing written.
/// - Sectors worked at `source` move to the resolved target, because `source`'s row no longer exists
///   to hold them (18 at 41, then 41 at 20, leaves 18 at 20).
///
/// One ARTCC's writes are serialised, so two concurrent saves can't build a loop between them.
pub async fn consolidate(
    pool: &PgPool,
    artcc: &str,
    source: &str,
    target: &str,
    updated_by: Option<&str>,
) -> Result<(), ApiError> {
    let db = |_| ApiError::Internal;
    let mut tx = pool.begin().await.map_err(db)?;
    sqlx::query("select pg_advisory_xact_lock(hashtext($1))")
        .bind(format!("sector_consolidation:{artcc}"))
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    let resolved: String = sqlx::query_scalar(
        "select target_sector_id from flow.sector_consolidation where artcc = $1 and sector_id = $2",
    )
    .bind(artcc)
    .bind(target)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db)?
    .unwrap_or_else(|| target.to_string());
    if resolved == source {
        return Err(ApiError::Conflict);
    }
    sqlx::query(
        "insert into flow.sector_consolidation (artcc, sector_id, target_sector_id, updated_by) \
         values ($1, $2, $3, $4) \
         on conflict (artcc, sector_id) do update \
         set target_sector_id = excluded.target_sector_id, updated_by = excluded.updated_by, \
             updated_at = now()",
    )
    .bind(artcc)
    .bind(source)
    .bind(&resolved)
    .bind(updated_by)
    .execute(&mut *tx)
    .await
    .map_err(db)?;
    sqlx::query(
        "update flow.sector_consolidation \
         set target_sector_id = $3, updated_by = $4, updated_at = now() \
         where artcc = $1 and target_sector_id = $2",
    )
    .bind(artcc)
    .bind(source)
    .bind(&resolved)
    .bind(updated_by)
    .execute(&mut *tx)
    .await
    .map_err(db)?;
    tx.commit().await.map_err(db)
}

/// Stop working `source` elsewhere; it gets its own row back. Releasing a sector that isn't
/// consolidated is a no-op.
pub async fn release(pool: &PgPool, artcc: &str, source: &str) -> Result<(), ApiError> {
    sqlx::query("delete from flow.sector_consolidation where artcc = $1 and sector_id = $2")
        .bind(artcc)
        .bind(source)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(())
}
