//! Sector consolidations (#723, migration 0128, `flow.sector_consolidation`). `load_all` builds the
//! [`SectorConsolidations`] cached in `AppState`; `consolidate`, `release` and `apply_batch` are the only writes
//! (`handlers::sector_consolidations`).

use std::collections::BTreeMap;

use sqlx::{PgPool, Postgres, Transaction};

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
    let mut tx = pool.begin().await.map_err(db)?;
    lock(&mut tx, artcc).await?;
    let changed = match consolidate_in(&mut tx, artcc, source, target, updated_by).await? {
        Ok(changed) => changed,
        Err(refusal) => return Ok(Err(refusal)),
    };
    tx.commit().await.map_err(db)?;
    Ok(Ok(changed))
}

/// One ARTCC's batch of changes (#794): each entry works its sector at the target given, or, for
/// `None`, gives it its own row back. All of it lands in one transaction or none of it does: the first
/// refusal (a self-reference, or a loop the batch would make, including one between its own entries)
/// returns before the commit, and the arrangement stays exactly as it was.
///
/// Releases apply first, then saves in sector order, each flattened like [`consolidate`]; upserting and
/// moving keeps the result flat whatever order the saves run in. Returns whether anything changed.
pub async fn apply_batch(
    pool: &PgPool,
    artcc: &str,
    entries: &BTreeMap<String, Option<String>>,
    updated_by: Option<&str>,
) -> Result<Result<bool, Refusal>, ApiError> {
    if entries
        .iter()
        .any(|(source, target)| target.as_deref() == Some(source.as_str()))
    {
        return Ok(Err(Refusal::SelfReference));
    }
    let mut tx = pool.begin().await.map_err(db)?;
    lock(&mut tx, artcc).await?;
    let mut changed = false;
    let releases: Vec<&str> = entries
        .iter()
        .filter(|(_, t)| t.is_none())
        .map(|(s, _)| s.as_str())
        .collect();
    changed |= release_in(&mut tx, artcc, &releases).await?;
    for (source, target) in entries.iter().filter_map(|(s, t)| Some((s, t.as_ref()?))) {
        match consolidate_in(&mut tx, artcc, source, target, updated_by).await? {
            Ok(saved) => changed |= saved,
            Err(refusal) => return Ok(Err(refusal)),
        }
    }
    tx.commit().await.map_err(db)?;
    Ok(Ok(changed))
}

fn db(_: sqlx::Error) -> ApiError {
    ApiError::Internal
}

/// Serialise this ARTCC's writes for the rest of the transaction.
async fn lock(tx: &mut Transaction<'_, Postgres>, artcc: &str) -> Result<(), ApiError> {
    sqlx::query("select pg_advisory_xact_lock(hashtext($1))")
        .bind(format!("sector_consolidation:{artcc}"))
        .execute(&mut **tx)
        .await
        .map_err(db)?;
    Ok(())
}

/// [`consolidate`]'s writes, inside a transaction that already holds the ARTCC's lock. `source` and
/// `target` differ. A refusal has written nothing.
async fn consolidate_in(
    tx: &mut Transaction<'_, Postgres>,
    artcc: &str,
    source: &str,
    target: &str,
    updated_by: Option<&str>,
) -> Result<Result<bool, Refusal>, ApiError> {
    let resolved: String = sqlx::query_scalar(
        "select target_sector_id from flow.sector_consolidation where artcc = $1 and sector_id = $2",
    )
    .bind(artcc)
    .bind(target)
    .fetch_optional(&mut **tx)
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
    .execute(&mut **tx)
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
    .execute(&mut **tx)
    .await
    .map_err(db)?
    .rows_affected();
    Ok(Ok(saved + moved > 0))
}

/// Gives each of `sources` its own row back, inside a caller's transaction, in one statement. Returns
/// whether any was released.
async fn release_in(
    tx: &mut Transaction<'_, Postgres>,
    artcc: &str,
    sources: &[&str],
) -> Result<bool, ApiError> {
    if sources.is_empty() {
        return Ok(false);
    }
    let deleted = sqlx::query(
        "delete from flow.sector_consolidation where artcc = $1 and sector_id = any($2)",
    )
    .bind(artcc)
    .bind(sources)
    .execute(&mut **tx)
    .await
    .map_err(db)?
    .rows_affected();
    Ok(deleted > 0)
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
