//! Writes ATC sector volumes (#594) to `flow.airspace_sector` (#821). The source file and its
//! parsing are `feed::sector_source`.
//!
//! The backend runs [`seed`] from the `airspace_sector_seed` job at start and then daily
//! (`jobs::spawn_airspace_sector_seed`). When every stored row already carries [`SOURCE_REF`] it
//! returns without a network call. Otherwise it fetches, parses, and replaces the whole table in one
//! transaction under [`SEED_LOCK_KEY`], so two replicas starting together can't both write, and a
//! failed fetch or write leaves the table as it was. Each row records `source` and `source_cycle`
//! (the commit), so the data can be traced and regenerated.

use std::{collections::BTreeSet, fmt, future::Future};

use sqlx::PgPool;

use crate::{
    feed::sector_source::{self, SOURCE, SOURCE_REF},
    repos::airspace_sectors,
};

/// Advisory-lock key, so two replicas that both find the table stale can't both replace it.
/// Distinct from the FAA and X-Plane seeds' keys: the seeds write different tables.
const SEED_LOCK_KEY: i64 = 0x004F_4953_5F53_4543; // "OIS_SEC"

/// What a [`seed`] call did.
#[derive(Debug, PartialEq)]
pub enum SeedOutcome {
    /// Every row already carries [`SOURCE_REF`]; nothing was fetched or written.
    Current,
    /// The table was replaced.
    Seeded {
        volumes: usize,
        artccs: usize,
        skipped: Vec<String>,
    },
}

impl fmt::Display for SeedOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Current => write!(f, "already at {SOURCE}@{SOURCE_REF}"),
            Self::Seeded {
                volumes,
                artccs,
                skipped,
            } => {
                write!(
                    f,
                    "loaded {volumes} volumes across {artccs} ARTCCs from {SOURCE}@{SOURCE_REF}; \
                     skipped {}",
                    skipped.len()
                )?;
                if !skipped.is_empty() {
                    write!(f, " ({})", skipped.join("; "))?;
                }
                Ok(())
            }
        }
    }
}

/// True when the table holds rows and every one is from [`SOURCE_REF`].
async fn is_current<'e, E>(db: E) -> Result<bool, sqlx::Error>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query_scalar(
        "select exists (select 1 from flow.airspace_sector) \
             and not exists (select 1 from flow.airspace_sector where source_cycle <> $1)",
    )
    .bind(SOURCE_REF)
    .fetch_one(db)
    .await
}

/// Bring `flow.airspace_sector` to [`SOURCE_REF`].
///
/// Unless `force` is set, a table that is already current returns [`SeedOutcome::Current`] without
/// calling `fetch`. Otherwise `fetch` runs first, outside any transaction, so a slow download holds
/// no connection or lock. Then, under [`SEED_LOCK_KEY`], the check runs again (another replica may
/// have just seeded) and the whole table is replaced in one transaction. A failed fetch, a document
/// that doesn't parse, a source with no valid volumes, or a failed write returns `Err` and leaves
/// the table untouched. Safe to call repeatedly and concurrently.
pub async fn seed<F, Fut>(pool: &PgPool, force: bool, fetch: F) -> Result<SeedOutcome, String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Vec<u8>, String>>,
{
    let db = |e: sqlx::Error| format!("airspace sector seed: {e}");
    if !force && is_current(pool).await.map_err(db)? {
        return Ok(SeedOutcome::Current);
    }

    let bytes = fetch().await?;
    // Parsing validates every ring (quadratic in its vertices), so it runs off the async workers.
    let parsed = tokio::task::spawn_blocking(move || sector_source::parse(&bytes))
        .await
        .map_err(|e| format!("airspace sector seed: parse task: {e}"))??;
    if parsed.volumes.is_empty() {
        return Err(format!(
            "{SOURCE}@{SOURCE_REF} has no valid volumes; the table was left as it was"
        ));
    }

    let mut tx = pool.begin().await.map_err(db)?;
    sqlx::query("select pg_advisory_xact_lock($1)")
        .bind(SEED_LOCK_KEY)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    // Read after taking the lock, so a replica that waited sees what the other one committed.
    if !force && is_current(&mut *tx).await.map_err(db)? {
        return Ok(SeedOutcome::Current);
    }
    sqlx::query("delete from flow.airspace_sector")
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    airspace_sectors::insert_volumes(&mut tx, &parsed.volumes, SOURCE, SOURCE_REF)
        .await
        .map_err(db)?;
    tx.commit().await.map_err(db)?;

    let artccs: BTreeSet<&str> = parsed.volumes.iter().map(|v| v.artcc.as_str()).collect();
    Ok(SeedOutcome::Seeded {
        volumes: parsed.volumes.len(),
        artccs: artccs.len(),
        skipped: parsed.skipped,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::*;
    use crate::feed::sector_source::tests::FIXTURE;

    /// A fetcher that serves [`FIXTURE`] and counts its calls.
    pub(crate) fn counting_fetch(
        calls: Arc<AtomicUsize>,
    ) -> impl FnOnce() -> std::future::Ready<Result<Vec<u8>, String>> {
        move || {
            calls.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Ok(FIXTURE.as_bytes().to_vec()))
        }
    }

    fn failing_fetch() -> std::future::Ready<Result<Vec<u8>, String>> {
        std::future::ready(Err("fetching: connection refused".into()))
    }

    /// `(artcc, volume_id, source_cycle)` for every row, ordered.
    async fn rows(pool: &PgPool) -> Vec<(String, String, String)> {
        sqlx::query_as(
            "select artcc, volume_id, source_cycle from flow.airspace_sector \
             order by artcc, volume_id",
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    /// Every row's content, so "untouched" can be asserted byte for byte.
    async fn snapshot(pool: &PgPool) -> Vec<(String, String, String, String, String)> {
        sqlx::query_as(
            "select artcc, volume_id, source, source_cycle, rings::text \
             from flow.airspace_sector order by artcc, volume_id",
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    fn row(artcc: &str, volume_id: &str, cycle: &str) -> (String, String, String) {
        (artcc.into(), volume_id.into(), cycle.into())
    }

    /// The job's detail on Admin → Jobs is this text, so it names every skipped feature and why.
    #[test]
    fn the_seeded_outcome_lists_each_skipped_feature() {
        let outcome = SeedOutcome::Seeded {
            volumes: 4,
            artccs: 3,
            skipped: vec![
                "ZTL 07007: inverted".into(),
                "ZOA 99001: unknown tier".into(),
            ],
        };
        assert_eq!(
            outcome.to_string(),
            "loaded 4 volumes across 3 ARTCCs from vtsd sectors.json@\
             f33ef73f21091e71456e45cfe9019ddf3ba76247; skipped 2 \
             (ZTL 07007: inverted; ZOA 99001: unknown tier)"
        );
        assert_eq!(
            SeedOutcome::Current.to_string(),
            "already at vtsd sectors.json@f33ef73f21091e71456e45cfe9019ddf3ba76247"
        );
    }

    /// AC1 at the repo level: an empty table is filled, stamped with the pinned provenance.
    #[sqlx::test]
    async fn an_empty_table_is_fetched_and_filled(pool: PgPool) {
        let calls = Arc::new(AtomicUsize::new(0));
        let outcome = seed(&pool, false, counting_fetch(calls.clone()))
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let SeedOutcome::Seeded {
            volumes,
            artccs,
            skipped,
        } = outcome
        else {
            panic!("expected a seed, got {outcome:?}");
        };
        assert_eq!((volumes, artccs, skipped.len()), (4, 3, 4));
        assert_eq!(
            rows(&pool).await,
            [
                row("ZAU", "02001", SOURCE_REF),
                row("ZAU", "50001", SOURCE_REF),
                row("ZDC", "01001", SOURCE_REF),
                row("ZNY", "07001", SOURCE_REF)
            ]
        );
        let sources: Vec<String> =
            sqlx::query_scalar("select distinct source from flow.airspace_sector")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(sources, ["vtsd sectors.json"]);
    }

    /// AC2: once the table is at `SOURCE_REF`, a second run makes no network call.
    #[sqlx::test]
    async fn a_current_table_is_not_fetched_again(pool: PgPool) {
        let calls = Arc::new(AtomicUsize::new(0));
        seed(&pool, false, counting_fetch(calls.clone()))
            .await
            .unwrap();
        let before = snapshot(&pool).await;

        let outcome = seed(&pool, false, counting_fetch(calls.clone()))
            .await
            .unwrap();

        assert_eq!(outcome, SeedOutcome::Current);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "the second run fetched");
        assert_eq!(snapshot(&pool).await, before);
    }

    /// A table from another commit is replaced whole: every row moves to `SOURCE_REF`, and an ARTCC
    /// the new data doesn't have is removed rather than left at the old cycle.
    #[sqlx::test]
    async fn a_table_from_another_commit_is_replaced_whole(pool: PgPool) {
        use crate::feed::sectors::tests::volume;
        airspace_sectors::replace_artcc(&pool, "ZDC", &[volume("ZDC", "09999")], SOURCE, "old")
            .await
            .unwrap();
        airspace_sectors::replace_artcc(&pool, "ZJX", &[volume("ZJX", "01001")], SOURCE, "old")
            .await
            .unwrap();

        let calls = Arc::new(AtomicUsize::new(0));
        seed(&pool, false, counting_fetch(calls.clone()))
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            rows(&pool).await,
            [
                row("ZAU", "02001", SOURCE_REF),
                row("ZAU", "50001", SOURCE_REF),
                row("ZDC", "01001", SOURCE_REF),
                row("ZNY", "07001", SOURCE_REF)
            ]
        );
    }

    /// One stale row is enough to fetch: a table that is mostly current but has an ARTCC left from
    /// another commit isn't "current".
    #[sqlx::test]
    async fn one_row_from_another_commit_makes_the_table_stale(pool: PgPool) {
        use crate::feed::sectors::tests::volume;
        let calls = Arc::new(AtomicUsize::new(0));
        seed(&pool, false, counting_fetch(calls.clone()))
            .await
            .unwrap();
        airspace_sectors::replace_artcc(&pool, "ZJX", &[volume("ZJX", "01001")], SOURCE, "old")
            .await
            .unwrap();

        seed(&pool, false, counting_fetch(calls.clone()))
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            rows(&pool).await,
            [
                row("ZAU", "02001", SOURCE_REF),
                row("ZAU", "50001", SOURCE_REF),
                row("ZDC", "01001", SOURCE_REF),
                row("ZNY", "07001", SOURCE_REF)
            ]
        );
    }

    /// `force` (the manual binary) fetches and rewrites even a current table.
    #[sqlx::test]
    async fn force_fetches_a_current_table(pool: PgPool) {
        let calls = Arc::new(AtomicUsize::new(0));
        seed(&pool, false, counting_fetch(calls.clone()))
            .await
            .unwrap();

        let outcome = seed(&pool, true, counting_fetch(calls.clone()))
            .await
            .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(matches!(outcome, SeedOutcome::Seeded { volumes: 4, .. }));
    }

    /// AC3 at the repo level: a failed fetch is an error and the stored rows are untouched.
    #[sqlx::test]
    async fn a_failed_fetch_errors_and_leaves_the_rows_untouched(pool: PgPool) {
        use crate::feed::sectors::tests::volume;
        airspace_sectors::replace_artcc(&pool, "ZDC", &[volume("ZDC", "09999")], SOURCE, "old")
            .await
            .unwrap();
        let before = snapshot(&pool).await;

        let err = seed(&pool, false, failing_fetch).await.unwrap_err();

        assert!(err.contains("connection refused"), "{err}");
        assert_eq!(snapshot(&pool).await, before);
    }

    /// A document with no valid volume (an error page that parses, a source gone empty) is refused
    /// rather than emptying the table.
    #[sqlx::test]
    async fn a_source_with_no_valid_volumes_errors_and_leaves_the_rows_untouched(pool: PgPool) {
        use crate::feed::sectors::tests::volume;
        airspace_sectors::replace_artcc(&pool, "ZDC", &[volume("ZDC", "09999")], SOURCE, "old")
            .await
            .unwrap();
        let before = snapshot(&pool).await;

        let empty = || std::future::ready(Ok(br#"{"features": []}"#.to_vec()));
        let err = seed(&pool, false, empty).await.unwrap_err();

        assert!(err.contains("no valid volumes"), "{err}");
        assert_eq!(snapshot(&pool).await, before);
    }

    /// A write that fails partway (here a duplicate `(artcc, volume_id)`) rolls back: the old rows
    /// are still there.
    #[sqlx::test]
    async fn a_failed_write_leaves_the_rows_untouched(pool: PgPool) {
        use crate::feed::sectors::tests::volume;
        airspace_sectors::replace_artcc(&pool, "ZDC", &[volume("ZDC", "09999")], SOURCE, "old")
            .await
            .unwrap();
        let before = snapshot(&pool).await;

        let doubled = FIXTURE.replace("\"full_id\": \"07001\"", "\"full_id\": \"01001\"");
        let doubled = doubled.replace("\"artcc\": \"ZNY\"", "\"artcc\": \"ZDC\"");
        let fetch = move || std::future::ready(Ok(doubled.into_bytes()));
        assert!(seed(&pool, false, fetch).await.is_err());

        assert_eq!(snapshot(&pool).await, before);
    }

    /// Two replicas starting on an empty table: both fetch, but the lock and the re-check under it
    /// mean one writes and the other finds the table current, with no duplicate or error.
    #[sqlx::test]
    async fn concurrent_seeds_write_once(pool: PgPool) {
        let calls = Arc::new(AtomicUsize::new(0));
        let (a, b) = tokio::join!(
            seed(&pool, false, counting_fetch(calls.clone())),
            seed(&pool, false, counting_fetch(calls.clone())),
        );
        let (a, b) = (a.unwrap(), b.unwrap());

        let seeded = [&a, &b]
            .iter()
            .filter(|o| matches!(o, SeedOutcome::Seeded { .. }))
            .count();
        assert_eq!(seeded, 1, "{a:?} / {b:?}");
        assert_eq!(rows(&pool).await.len(), 4);
    }
}
