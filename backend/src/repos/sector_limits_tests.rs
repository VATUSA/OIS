//! VATUSA/OIS#722: `flow.sector_limit` (migration 0127) and its repo — the table refuses a limit that
//! is not positive, a write replaces in place, a delete touches one `(artcc, sector_id)` only, and the
//! refresh job carries a row written behind the cache's back into it.

use std::sync::Arc;

use arc_swap::ArcSwap;
use sqlx::PgPool;

use crate::{
    feed::sector_limits::SectorLimits,
    job_registry::JobRegistry,
    jobs::spawn_sector_limits_refresh,
    repos::sector_limits::{delete, get, load_all, upsert},
    scope_test_support::seed_user,
};

fn key(artcc: &str, sector_id: &str) -> (String, String) {
    (artcc.to_string(), sector_id.to_string())
}

async fn updated_by(pool: &PgPool, artcc: &str, sector_id: &str) -> Option<String> {
    sqlx::query_scalar(
        "select updated_by from flow.sector_limit where artcc = $1 and sector_id = $2",
    )
    .bind(artcc)
    .bind(sector_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// The table is the last line: zero and negatives never reach it, whatever a caller does.
#[sqlx::test]
async fn the_table_refuses_a_limit_that_is_not_positive(pool: PgPool) {
    for bad in [0, -1, -10] {
        let raw = sqlx::query(
            "insert into flow.sector_limit (artcc, sector_id, limit_value) values ('ZDC', '010', $1)",
        )
        .bind(bad)
        .execute(&pool)
        .await;
        assert!(raw.is_err(), "the check accepted {bad}");
        assert!(upsert(&pool, "ZDC", "010", bad, None).await.is_err());
    }
    assert!(load_all(&pool).await.unwrap().is_empty());

    upsert(&pool, "ZDC", "010", 1, None).await.unwrap();
    assert_eq!(
        get(&pool, "ZDC", "010").await.unwrap(),
        Some(1),
        "1 is positive"
    );
}

#[sqlx::test]
async fn a_missing_override_reads_none(pool: PgPool) {
    upsert(&pool, "ZDC", "011", 12, None).await.unwrap();
    assert_eq!(get(&pool, "ZDC", "010").await.unwrap(), None);
    assert_eq!(get(&pool, "ZNY", "011").await.unwrap(), None);
}

#[sqlx::test]
async fn upsert_replaces_in_place(pool: PgPool) {
    let (first, second) = (seed_user(&pool).await, seed_user(&pool).await);
    upsert(&pool, "ZDC", "010", 14, Some(&first)).await.unwrap();
    upsert(&pool, "ZDC", "010", 7, Some(&second)).await.unwrap();

    let rows: i64 = sqlx::query_scalar("select count(*) from flow.sector_limit")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 1);
    assert_eq!(get(&pool, "ZDC", "010").await.unwrap(), Some(7));
    assert_eq!(updated_by(&pool, "ZDC", "010").await, Some(second));
}

/// One surviving neighbour per predicate: the same ARTCC's other sector (drop `sector_id = $2` and
/// it goes) and the same sector id in another ARTCC (drop `artcc = $1` and it goes).
#[sqlx::test]
async fn delete_removes_only_the_targeted_sector(pool: PgPool) {
    upsert(&pool, "ZDC", "010", 12, None).await.unwrap();
    upsert(&pool, "ZDC", "011", 13, None).await.unwrap();
    upsert(&pool, "ZNY", "010", 14, None).await.unwrap();

    delete(&pool, "ZDC", "010").await.unwrap();

    let expected: SectorLimits = [(key("ZDC", "011"), 13), (key("ZNY", "010"), 14)].into();
    assert_eq!(load_all(&pool).await.unwrap(), expected);
}

#[sqlx::test]
async fn deleting_a_missing_override_changes_nothing(pool: PgPool) {
    upsert(&pool, "ZDC", "011", 13, None).await.unwrap();
    delete(&pool, "ZDC", "010").await.unwrap();
    assert_eq!(
        load_all(&pool).await.unwrap(),
        [(key("ZDC", "011"), 13)].into()
    );
}

#[sqlx::test]
async fn load_all_round_trips_every_override(pool: PgPool) {
    assert!(load_all(&pool).await.unwrap().is_empty());
    upsert(&pool, "ZDC", "010", 12, None).await.unwrap();
    upsert(&pool, "ZDC", "020", 18, None).await.unwrap();
    upsert(&pool, "ZNY", "010", 9, None).await.unwrap();

    let expected: SectorLimits = [
        (key("ZDC", "010"), 12),
        (key("ZDC", "020"), 18),
        (key("ZNY", "010"), 9),
    ]
    .into();
    assert_eq!(load_all(&pool).await.unwrap(), expected);
}

/// Deleting the account that set a limit keeps the limit; only the attribution goes.
#[sqlx::test]
async fn deleting_the_editor_keeps_the_limit(pool: PgPool) {
    let user = seed_user(&pool).await;
    upsert(&pool, "ZDC", "010", 14, Some(&user)).await.unwrap();

    sqlx::query("delete from identity.users where id = $1")
        .bind(&user)
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(get(&pool, "ZDC", "010").await.unwrap(), Some(14));
    assert_eq!(updated_by(&pool, "ZDC", "010").await, None);
}

/// The groups whose presets carry `flow` hold the permission; CONTROLLER (#730) and USER do not.
#[sqlx::test]
async fn the_five_flow_groups_hold_the_update_permission(pool: PgPool) {
    let holders: Vec<String> = sqlx::query_scalar(
        "select role_name from access.role_permissions \
         where permission_name = 'flow.sector_limits.update' order by role_name",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(holders, ["AEC", "DCC_STAFF", "EC", "NTMO", "VATUSA_STAFF"]);

    let roles: Vec<String> =
        sqlx::query_scalar("select name from access.roles where name in ('CONTROLLER', 'USER')")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        roles.len(),
        2,
        "both roles exist, so their absence above means something"
    );
}

/// Another replica's write reaches this one's cache through the job, not a request.
#[sqlx::test]
async fn the_refresh_job_loads_a_row_written_behind_the_cache(pool: PgPool) {
    let cache = Arc::new(ArcSwap::from_pointee(SectorLimits::new()));
    upsert(&pool, "ZDC", "010", 14, None).await.unwrap();

    spawn_sector_limits_refresh(Arc::new(JobRegistry::new()), pool, cache.clone());

    for _ in 0..100 {
        if !cache.load().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(**cache.load(), [(key("ZDC", "010"), 14)].into());
}
