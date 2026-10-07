//! VATUSA/OIS#723: `flow.sector_consolidation` (migration 0128) and its repo — self-reference and loops
//! are refused without a write, chains are flattened on every write, a release touches one
//! `(artcc, sector_id)` only, and the refresh job carries a row written behind the cache into it.
//!
//! Every destructive or rewriting WHERE gets one surviving neighbour per predicate.

use std::sync::Arc;

use arc_swap::ArcSwap;
use sqlx::PgPool;

use crate::{
    feed::sector_consolidations::SectorConsolidations,
    job_registry::JobRegistry,
    jobs::spawn_sector_consolidations_refresh,
    repos::sector_consolidations::{Refusal, consolidate, load_all, release},
    scope_test_support::seed_user,
};

/// `(artcc, source, target)` triples as the cache holds them.
fn arrangement(rows: &[(&str, &str, &str)]) -> SectorConsolidations {
    rows.iter()
        .map(|(a, s, t)| ((a.to_string(), s.to_string()), t.to_string()))
        .collect()
}

async fn ok(pool: &PgPool, artcc: &str, source: &str, target: &str) -> bool {
    consolidate(pool, artcc, source, target, None)
        .await
        .unwrap()
        .unwrap()
}

/// `(updated_by, updated_at)` of one row.
async fn stamp(pool: &PgPool, artcc: &str, sector_id: &str) -> (Option<String>, String) {
    sqlx::query_as(
        "select updated_by, updated_at::text from flow.sector_consolidation \
         where artcc = $1 and sector_id = $2",
    )
    .bind(artcc)
    .bind(sector_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn backdate(pool: &PgPool) {
    sqlx::query("update flow.sector_consolidation set updated_at = '2020-01-01T00:00:00Z'")
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test]
async fn a_consolidation_is_stored_and_loaded(pool: PgPool) {
    let user = seed_user(&pool).await;
    assert!(load_all(&pool).await.unwrap().is_empty());
    assert_eq!(
        consolidate(&pool, "ZDC", "018", "041", Some(&user))
            .await
            .unwrap(),
        Ok(true)
    );
    assert_eq!(
        load_all(&pool).await.unwrap(),
        arrangement(&[("ZDC", "018", "041")])
    );
    assert_eq!(stamp(&pool, "ZDC", "018").await.0, Some(user));
}

/// AC4: a sector can't be worked at itself — refused before the table, and the table's own check
/// refuses it too.
#[sqlx::test]
async fn a_self_reference_is_refused_without_a_write(pool: PgPool) {
    ok(&pool, "ZDC", "018", "041").await;
    assert_eq!(
        consolidate(&pool, "ZDC", "041", "041", None).await.unwrap(),
        Err(Refusal::SelfReference)
    );
    assert_eq!(
        consolidate(&pool, "ZDC", "018", "018", None).await.unwrap(),
        Err(Refusal::SelfReference),
        "nor an existing source"
    );
    assert_eq!(
        load_all(&pool).await.unwrap(),
        arrangement(&[("ZDC", "018", "041")])
    );

    let raw = sqlx::query(
        "insert into flow.sector_consolidation (artcc, sector_id, target_sector_id) \
         values ('ZDC', '050', '050')",
    )
    .execute(&pool)
    .await;
    assert!(raw.is_err(), "the table's check accepted a self-reference");
}

/// AC4: 018 at 041, then 041 at 018, is a loop. Refused, and the existing arrangement stands to its
/// timestamp.
#[sqlx::test]
async fn a_loop_is_refused_without_a_write(pool: PgPool) {
    ok(&pool, "ZDC", "018", "041").await;
    backdate(&pool).await;
    let before = stamp(&pool, "ZDC", "018").await;

    assert_eq!(
        consolidate(&pool, "ZDC", "041", "018", None).await.unwrap(),
        Err(Refusal::Loop)
    );
    assert_eq!(
        load_all(&pool).await.unwrap(),
        arrangement(&[("ZDC", "018", "041")])
    );
    assert_eq!(stamp(&pool, "ZDC", "018").await, before);
}

/// The loop check is per ARTCC: ZNY's 041 being worked at 018 doesn't make ZDC's 018-at-041 a loop.
#[sqlx::test]
async fn another_artccs_arrangement_is_not_a_loop(pool: PgPool) {
    ok(&pool, "ZNY", "041", "018").await;
    assert!(ok(&pool, "ZDC", "018", "041").await);
    assert_eq!(
        load_all(&pool).await.unwrap(),
        arrangement(&[("ZNY", "041", "018"), ("ZDC", "018", "041")])
    );
}

/// AC3, the issue's example: 018 at 041, then 041 at 020, leaves 018 at 020 — 041's row no longer
/// exists to hold it. Neighbours, one per predicate of the re-pointing UPDATE: ZNY's 018 at 041
/// (`artcc`) and ZDC's 030 at 050 (`target_sector_id`) are left exactly as they were.
#[sqlx::test]
async fn consolidating_a_target_moves_its_sources_with_it(pool: PgPool) {
    ok(&pool, "ZDC", "018", "041").await;
    ok(&pool, "ZNY", "018", "041").await;
    ok(&pool, "ZDC", "030", "050").await;
    backdate(&pool).await;
    let (zny, zdc_030) = (
        stamp(&pool, "ZNY", "018").await,
        stamp(&pool, "ZDC", "030").await,
    );

    assert!(ok(&pool, "ZDC", "041", "020").await);
    assert_eq!(
        load_all(&pool).await.unwrap(),
        arrangement(&[
            ("ZDC", "018", "020"),
            ("ZDC", "041", "020"),
            ("ZNY", "018", "041"),
            ("ZDC", "030", "050"),
        ])
    );
    assert_eq!(stamp(&pool, "ZNY", "018").await, zny);
    assert_eq!(stamp(&pool, "ZDC", "030").await, zdc_030);
}

/// AC3: consolidating onto a sector that is itself worked elsewhere resolves to where it is worked
/// (020 onto 018 while 018 is at 041 saves 020 at 041), and onto a sector that is already a target
/// joins it — flat either way.
#[sqlx::test]
async fn consolidating_onto_a_source_resolves_to_its_target(pool: PgPool) {
    ok(&pool, "ZDC", "018", "041").await;
    assert!(ok(&pool, "ZDC", "020", "018").await);
    assert!(ok(&pool, "ZDC", "030", "041").await);
    assert_eq!(
        load_all(&pool).await.unwrap(),
        arrangement(&[
            ("ZDC", "018", "041"),
            ("ZDC", "020", "041"),
            ("ZDC", "030", "041"),
        ])
    );
}

/// Moving a source to a new target replaces its row; saving the stored arrangement writes nothing —
/// not even its timestamp.
#[sqlx::test]
async fn an_unchanged_save_writes_nothing(pool: PgPool) {
    let user = seed_user(&pool).await;
    ok(&pool, "ZDC", "018", "041").await;
    backdate(&pool).await;
    let before = stamp(&pool, "ZDC", "018").await;

    assert_eq!(
        consolidate(&pool, "ZDC", "018", "041", Some(&user))
            .await
            .unwrap(),
        Ok(false)
    );
    assert_eq!(stamp(&pool, "ZDC", "018").await, before);

    assert_eq!(
        consolidate(&pool, "ZDC", "018", "050", Some(&user))
            .await
            .unwrap(),
        Ok(true)
    );
    assert_eq!(
        load_all(&pool).await.unwrap(),
        arrangement(&[("ZDC", "018", "050")])
    );
    assert_eq!(stamp(&pool, "ZDC", "018").await.0, Some(user));
}

/// AC7, one surviving neighbour per predicate of the release: ZDC's other source 020 (drop
/// `sector_id = $2` and it goes) and ZNY's 018 (drop `artcc = $1` and it goes). 041, a target,
/// keeps the sources worked at it.
#[sqlx::test]
async fn release_removes_only_the_named_source(pool: PgPool) {
    ok(&pool, "ZDC", "018", "041").await;
    ok(&pool, "ZDC", "020", "041").await;
    ok(&pool, "ZNY", "018", "041").await;

    assert!(release(&pool, "ZDC", "018").await.unwrap());
    assert_eq!(
        load_all(&pool).await.unwrap(),
        arrangement(&[("ZDC", "020", "041"), ("ZNY", "018", "041")])
    );

    assert!(
        !release(&pool, "ZDC", "041").await.unwrap(),
        "a target isn't a source"
    );
    assert!(
        !release(&pool, "ZDC", "018").await.unwrap(),
        "already released"
    );
    assert_eq!(
        load_all(&pool).await.unwrap(),
        arrangement(&[("ZDC", "020", "041"), ("ZNY", "018", "041")])
    );
}

/// Deleting the account that made a consolidation keeps it; only the attribution goes.
#[sqlx::test]
async fn deleting_the_editor_keeps_the_consolidation(pool: PgPool) {
    let user = seed_user(&pool).await;
    consolidate(&pool, "ZDC", "018", "041", Some(&user))
        .await
        .unwrap()
        .unwrap();
    sqlx::query("delete from identity.users where id = $1")
        .bind(&user)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        load_all(&pool).await.unwrap(),
        arrangement(&[("ZDC", "018", "041")])
    );
    assert_eq!(stamp(&pool, "ZDC", "018").await.0, None);
}

/// Owner decision: the new permission is seeded to exactly the groups holding
/// `flow.sector_limits.update` — not CONTROLLER, which is the owner's call.
#[sqlx::test]
async fn the_update_permission_mirrors_the_sector_limit_groups(pool: PgPool) {
    let holders = |permission: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, String>(
                "select role_name from access.role_permissions \
                 where permission_name = $1 order by role_name",
            )
            .bind(permission)
            .fetch_all(&pool)
            .await
            .unwrap()
        }
    };
    let consolidations = holders("flow.sector_consolidations.update").await;
    assert_eq!(
        consolidations,
        ["AEC", "DCC_STAFF", "EC", "NTMO", "VATUSA_STAFF"]
    );
    assert_eq!(consolidations, holders("flow.sector_limits.update").await);

    let known: i64 = sqlx::query_scalar(
        "select count(*) from access.permissions where name = 'flow.sector_consolidations.update'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(known, 1);
}

/// Another replica's write reaches this one's cache through the job, not a request.
#[sqlx::test]
async fn the_refresh_job_loads_a_row_written_behind_the_cache(pool: PgPool) {
    let cache = Arc::new(ArcSwap::from_pointee(SectorConsolidations::new()));
    ok(&pool, "ZDC", "018", "041").await;

    spawn_sector_consolidations_refresh(Arc::new(JobRegistry::new()), pool, cache.clone());

    for _ in 0..100 {
        if !cache.load().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(**cache.load(), arrangement(&[("ZDC", "018", "041")]));
}
