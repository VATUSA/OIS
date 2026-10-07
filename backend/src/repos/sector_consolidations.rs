//! Sector consolidations (#723, migration 0128, `flow.sector_consolidation`). `load_all` builds the
//! [`SectorConsolidations`] cached in `AppState`; `consolidate` and `release` are the only writes
//! (`handlers::sector_consolidations`).

use sqlx::PgPool;

use crate::{errors::ApiError, feed::sector_consolidations::SectorConsolidations};

/// Why a consolidation was refused. Nothing is written in either case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// `source` and `target` are the same sector.
    SelfReference,
    /// `target` is worked at `source` (or resolves to it), so saving would make a loop.
    Loop,
}

/// Every stored consolidation.
pub async fn load_all(pool: &PgPool) -> Result<SectorConsolidations, ApiError> {
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
/// - `source == target` is refused ([`Refusal::SelfReference`]).
/// - A target that is itself worked elsewhere resolves to where it is worked (20 at 41 while 41 is
///   worked at 50 saves 20 at 50).
/// - If that resolves back to `source` (a at b, then b at a) it is a loop: [`Refusal::Loop`].
/// - Sectors worked at `source` move to the resolved target, because `source`'s row no longer exists
///   to hold them (18 at 41, then 41 at 20, leaves 18 at 20).
///
/// One ARTCC's writes are serialised, so two concurrent saves can't build a loop between them.
/// Returns whether anything changed: saving the arrangement that is already stored writes nothing.
pub async fn consolidate(
    pool: &PgPool,
    artcc: &str,
    source: &str,
    target: &str,
    updated_by: Option<&str>,
) -> Result<Result<bool, Refusal>, ApiError> {
    if source == target {
        return Ok(Err(Refusal::SelfReference));
    }
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
        return Ok(Err(Refusal::Loop));
    }
    let saved = sqlx::query(
        "insert into flow.sector_consolidation (artcc, sector_id, target_sector_id, updated_by) \
         values ($1, $2, $3, $4) \
         on conflict (artcc, sector_id) do update \
         set target_sector_id = excluded.target_sector_id, updated_by = excluded.updated_by, \
             updated_at = now() \
         where flow.sector_consolidation.target_sector_id <> excluded.target_sector_id",
    )
    .bind(artcc)
    .bind(source)
    .bind(&resolved)
    .bind(updated_by)
    .execute(&mut *tx)
    .await
    .map_err(db)?
    .rows_affected();
    let moved = sqlx::query(
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
    .map_err(db)?
    .rows_affected();
    tx.commit().await.map_err(db)?;
    Ok(Ok(saved + moved > 0))
}

/// Stop working `source` elsewhere; it gets its own row back. Touches only that `(artcc, sector_id)`,
/// and returns whether there was one to release.
pub async fn release(pool: &PgPool, artcc: &str, source: &str) -> Result<bool, ApiError> {
    let deleted =
        sqlx::query("delete from flow.sector_consolidation where artcc = $1 and sector_id = $2")
            .bind(artcc)
            .bind(source)
            .execute(pool)
            .await
            .map_err(|_| ApiError::Internal)?
            .rows_affected();
    Ok(deleted > 0)
}
