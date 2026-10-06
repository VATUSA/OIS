//! VATUSA/OIS#722: the sector limit routes, through the real router — session resolution,
//! `RequirePermission`, the handler's facility scope and its write rules all on the path.
//!
//! Limits are literals (`10`, `14`), never `DEFAULT_LIMIT`: a fixture derived from the constant under
//! test passes for every value of it.

use std::{collections::HashMap, sync::Arc};

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::broadcast::Receiver;

use crate::{
    feed::sectors::{SectorTable, SectorVolume, tests::volume},
    realtime::{WsEvent, topic},
    repos::sector_limits as repo,
    scope_test_support::{grant, seed_user, send, send_json, session_cookie, test_state},
    state::AppState,
};

const READ: &str = "flow.sectors.read";
const UPDATE: &str = "flow.sector_limits.update";

fn tiered(artcc: &str, volume_id: &str, tier: &str) -> SectorVolume {
    SectorVolume {
        tier: tier.into(),
        ..volume(artcc, volume_id)
    }
}

/// ZDC has sectors 010 (two volumes) and 020 (first volume `high`, second `low`), stored out of
/// order; ZNY has 010 and 030. ZOB has none.
fn state(pool: PgPool) -> AppState {
    let state = test_state(pool, HashMap::new());
    state.airspace_sectors.store(Arc::new(SectorTable {
        volumes: vec![
            tiered("ZDC", "02001", "high"),
            tiered("ZDC", "02002", "low"),
            volume("ZDC", "01001"),
            volume("ZDC", "01002"),
            volume("ZNY", "01001"),
            volume("ZNY", "03001"),
        ],
    }));
    state
}

/// A signed-in user holding `flow.sectors.read` nationally and `update` at `update_at` (`None` for
/// no update grant, `Some(None)` for national).
async fn user(pool: &PgPool, update_at: Option<Option<&str>>) -> (String, String) {
    let id = seed_user(pool).await;
    grant(pool, &id, READ, None).await;
    if let Some(artcc) = update_at {
        grant(pool, &id, UPDATE, artcc).await;
    }
    let cookie = session_cookie(pool, &id).await;
    (id, cookie)
}

async fn put_raw(state: &AppState, uri: &str, cookie: &str, body: &str) -> (StatusCode, Value) {
    use tower::ServiceExt;

    let request = http::Request::builder()
        .method(Method::PUT)
        .uri(uri)
        .header(http::header::COOKIE, cookie)
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap();
    let response = crate::router::build_router(state.clone())
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn put(state: &AppState, uri: &str, cookie: &str, limit: i32) -> (StatusCode, Value) {
    put_raw(state, uri, cookie, &json!({ "limit": limit }).to_string()).await
}

async fn get(state: &AppState, artcc: &str, cookie: &str) -> Value {
    let uri = format!("/api/v1/flow/sector-limits/{artcc}");
    let (status, body) = send_json(state, Method::GET, &uri, cookie).await;
    assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    body
}

fn row(sector_id: &str, tier: &str, limit: i32, overridden: bool) -> Value {
    json!({ "sector_id": sector_id, "tier": tier, "limit": limit, "overridden": overridden })
}

/// How many events arrived since the last drain, and whether all were `flow.sector_limits`.
fn drain(rx: &mut Receiver<WsEvent>) -> usize {
    let mut n = 0;
    while let Ok(event) = rx.try_recv() {
        assert_eq!(event.topic, topic::SECTOR_LIMITS);
        n += 1;
    }
    n
}

/// Seed an override in the DB and the cache, as a write by `by` at a fixed past instant — so a later
/// rewrite shows in `updated_at` as well as `updated_by`.
async fn seed_override(state: &AppState, artcc: &str, sector_id: &str, limit: i32, by: &str) {
    let pool = state.db.as_ref().unwrap();
    repo::upsert(pool, artcc, sector_id, limit, Some(by))
        .await
        .unwrap();
    sqlx::query(
        "update flow.sector_limit set updated_at = '2020-01-01T00:00:00Z' \
         where artcc = $1 and sector_id = $2",
    )
    .bind(artcc)
    .bind(sector_id)
    .execute(pool)
    .await
    .unwrap();
    state
        .sector_limits
        .store(Arc::new(repo::load_all(pool).await.unwrap()));
}

/// `(limit_value, updated_by, updated_at)` of one stored row, `None` without one.
async fn stored(
    pool: &PgPool,
    artcc: &str,
    sector_id: &str,
) -> Option<(i32, Option<String>, String)> {
    sqlx::query_as(
        "select limit_value, updated_by, updated_at::text from flow.sector_limit \
         where artcc = $1 and sector_id = $2",
    )
    .bind(artcc)
    .bind(sector_id)
    .fetch_optional(pool)
    .await
    .unwrap()
}

const ZDC_010: &str = "/api/v1/flow/sector-limits/ZDC/010";
const ZNY_010: &str = "/api/v1/flow/sector-limits/ZNY/010";

#[sqlx::test]
async fn a_sector_with_no_override_reads_ten(pool: PgPool) {
    let state = state(pool.clone());
    let (_, cookie) = user(&pool, Some(Some("ZDC"))).await;

    // Lower-case in the path, one sector per id, ordered by id, tier from the first volume.
    assert_eq!(
        get(&state, "zdc", &cookie).await,
        json!({
            "artcc": "ZDC",
            "default_limit": 10,
            "editable": true,
            "sectors": [row("010", "low", 10, false), row("020", "high", 10, false)],
        })
    );
}

#[sqlx::test]
async fn an_override_shows_on_its_own_sector_only(pool: PgPool) {
    let state = state(pool.clone());
    let (id, cookie) = user(&pool, None).await;
    seed_override(&state, "ZNY", "010", 14, &id).await;

    assert_eq!(
        get(&state, "ZNY", &cookie).await["sectors"],
        json!([row("010", "low", 14, true), row("030", "low", 10, false)])
    );
    assert_eq!(
        get(&state, "ZDC", &cookie).await["sectors"],
        json!([row("010", "low", 10, false), row("020", "high", 10, false)]),
        "the same sector id in another ARTCC"
    );
}

/// The UI names the gap; an ARTCC with no sector data is not a 404.
#[sqlx::test]
async fn an_artcc_without_sector_data_answers_with_no_sectors(pool: PgPool) {
    let state = state(pool.clone());
    let (_, cookie) = user(&pool, Some(None)).await;
    assert_eq!(
        get(&state, "ZOB", &cookie).await,
        json!({ "artcc": "ZOB", "default_limit": 10, "editable": true, "sectors": [] })
    );
}

#[sqlx::test]
async fn reading_needs_flow_sectors_read(pool: PgPool) {
    let state = state(pool.clone());
    let id = seed_user(&pool).await;
    grant(&pool, &id, UPDATE, None).await;
    let cookie = session_cookie(&pool, &id).await;

    let uri = "/api/v1/flow/sector-limits/ZDC";
    assert_eq!(
        send(&state, Method::GET, uri, &cookie, None).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(&state, Method::GET, uri, "", None).await,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test]
async fn editable_follows_the_callers_facility(pool: PgPool) {
    let state = state(pool.clone());
    let (_, zdc) = user(&pool, Some(Some("ZDC"))).await;
    let (_, national) = user(&pool, Some(None)).await;
    let (_, reader) = user(&pool, None).await;

    for (cookie, artcc, editable) in [
        (&zdc, "ZDC", true),
        (&zdc, "ZNY", false),
        (&national, "ZDC", true),
        (&national, "ZNY", true),
        (&reader, "ZDC", false),
    ] {
        assert_eq!(
            get(&state, artcc, cookie).await["editable"],
            json!(editable),
            "{artcc}"
        );
    }
}

/// The AC: a TMU sets their own facility's limits and is refused a neighbour's — through the router,
/// so a handler that stopped checking scope fails here.
#[sqlx::test]
async fn a_tmu_sets_their_own_facility_but_not_a_neighbours(pool: PgPool) {
    let state = state(pool.clone());
    let (zdc_id, zdc) = user(&pool, Some(Some("ZDC"))).await;

    let (status, body) = put(&state, "/api/v1/flow/sector-limits/zdc/010", &zdc, 14).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, row("010", "low", 14, true));
    let (limit, by, _) = stored(&pool, "ZDC", "010").await.unwrap();
    assert_eq!((limit, by), (14, Some(zdc_id)));
    assert_eq!(
        get(&state, "ZDC", &zdc).await["sectors"][0],
        row("010", "low", 14, true)
    );

    for uri in [ZNY_010, "/api/v1/flow/sector-limits/zny/010"] {
        let (status, _) = put(&state, uri, &zdc, 14).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{uri}");
    }
    assert_eq!(stored(&pool, "ZNY", "010").await, None);
}

/// A neighbour's refusal touches nothing, including the reset value, which would delete.
#[sqlx::test]
async fn a_refused_neighbour_keeps_its_override(pool: PgPool) {
    let state = state(pool.clone());
    let (zdc_id, zdc) = user(&pool, Some(Some("ZDC"))).await;
    seed_override(&state, "ZNY", "010", 12, &zdc_id).await;
    let before = stored(&pool, "ZNY", "010").await;
    let mut rx = state.events.subscribe();

    for limit in [10, 14] {
        let (status, _) = put(&state, ZNY_010, &zdc, limit).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{limit}");
        assert_eq!(stored(&pool, "ZNY", "010").await, before, "{limit}");
    }
    assert_eq!(drain(&mut rx), 0);
}

#[sqlx::test]
async fn a_national_grant_sets_any_facility(pool: PgPool) {
    let state = state(pool.clone());
    let (_, national) = user(&pool, Some(None)).await;

    for (uri, artcc) in [(ZDC_010, "ZDC"), (ZNY_010, "ZNY")] {
        let (status, body) = put(&state, uri, &national, 14).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        assert_eq!(repo::get(&pool, artcc, "010").await.unwrap(), Some(14));
    }
}

#[sqlx::test]
async fn writing_needs_flow_sector_limits_update(pool: PgPool) {
    let state = state(pool.clone());
    let (_, reader) = user(&pool, None).await;

    for cookie in [reader.as_str(), ""] {
        let (status, _) = put(&state, ZDC_010, cookie, 14).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(stored(&pool, "ZDC", "010").await, None);
}

/// The AC: zero, negative, non-numeric, missing and unchanged all cancel — nothing written, nothing
/// deleted, the existing override untouched down to its timestamp, and nobody told to recolour.
#[sqlx::test]
async fn invalid_or_unchanged_input_writes_nothing_and_keeps_the_override(pool: PgPool) {
    let state = state(pool.clone());
    let author = seed_user(&pool).await;
    let (_, zdc) = user(&pool, Some(Some("ZDC"))).await;
    seed_override(&state, "ZDC", "010", 14, &author).await;
    let before = stored(&pool, "ZDC", "010").await;
    assert_eq!(before.as_ref().map(|r| r.0), Some(14));
    let mut rx = state.events.subscribe();

    for (body, refused_as) in [
        (r#"{"limit":0}"#, Some(StatusCode::BAD_REQUEST)),
        (r#"{"limit":-3}"#, Some(StatusCode::BAD_REQUEST)),
        (r#"{"limit":"abc"}"#, None),
        (r#"{"limit":1.5}"#, None),
        (r#"{"limit":null}"#, None),
        (r#"{}"#, None),
        ("abc", None),
    ] {
        let (status, _) = put_raw(&state, ZDC_010, &zdc, body).await;
        match refused_as {
            Some(expected) => assert_eq!(status, expected, "{body}"),
            None => assert!(status.is_client_error(), "{body}: {status}"),
        }
        assert_eq!(stored(&pool, "ZDC", "010").await, before, "{body}");
    }

    let (status, body) = put(&state, ZDC_010, &zdc, 14).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, row("010", "low", 14, true));
    assert_eq!(stored(&pool, "ZDC", "010").await, before, "unchanged");

    assert_eq!(drain(&mut rx), 0);
    assert_eq!(
        **state.sector_limits.load(),
        HashMap::from([(("ZDC".to_string(), "010".to_string()), 14)])
    );
}

/// "Unchanged" is judged against the row, not this replica's cache, in both directions.
#[sqlx::test]
async fn the_unchanged_check_reads_the_row_not_the_cache(pool: PgPool) {
    let state = state(pool.clone());
    let author = seed_user(&pool).await;
    let (_, zdc) = user(&pool, Some(Some("ZDC"))).await;
    let mut rx = state.events.subscribe();

    // Another replica wrote 14; this cache never heard. Sending 14 changes nothing.
    seed_override(&state, "ZDC", "010", 14, &author).await;
    state.sector_limits.store(Arc::default());
    let before = stored(&pool, "ZDC", "010").await;
    let (status, _) = put(&state, ZDC_010, &zdc, 14).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stored(&pool, "ZDC", "010").await, before);
    assert_eq!(drain(&mut rx), 0);

    // Another replica reset 020; this cache still reads 15. Sending 15 is a real change.
    state.sector_limits.store(Arc::new(HashMap::from([(
        ("ZDC".to_string(), "020".to_string()),
        15,
    )])));
    let (status, _) = put(&state, "/api/v1/flow/sector-limits/ZDC/020", &zdc, 15).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(repo::get(&pool, "ZDC", "020").await.unwrap(), Some(15));
    assert_eq!(drain(&mut rx), 1);
}

/// Setting the default removes the override rather than storing a 10 that reads "overridden"
/// forever; with nothing stored it does nothing. Neighbours keep theirs.
#[sqlx::test]
async fn the_default_is_the_reset(pool: PgPool) {
    let state = state(pool.clone());
    let author = seed_user(&pool).await;
    let (_, zdc) = user(&pool, Some(Some("ZDC"))).await;
    let (_, national) = user(&pool, Some(None)).await;
    seed_override(&state, "ZDC", "020", 15, &author).await;
    seed_override(&state, "ZNY", "010", 16, &author).await;
    seed_override(&state, "ZDC", "010", 14, &author).await;
    let mut rx = state.events.subscribe();

    let (status, body) = put(&state, ZDC_010, &zdc, 10).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, row("010", "low", 10, false));
    assert_eq!(stored(&pool, "ZDC", "010").await, None);
    assert_eq!(drain(&mut rx), 1);
    assert_eq!(
        get(&state, "ZDC", &zdc).await["sectors"],
        json!([row("010", "low", 10, false), row("020", "high", 15, true)])
    );
    assert_eq!(
        get(&state, "ZNY", &national).await["sectors"][0],
        row("010", "low", 16, true)
    );

    // Again, with no row: a no-op.
    let (status, body) = put(&state, ZDC_010, &zdc, 10).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, row("010", "low", 10, false));
    assert_eq!(stored(&pool, "ZDC", "010").await, None);
    assert_eq!(drain(&mut rx), 0);
}

#[sqlx::test]
async fn a_sector_outside_the_artccs_dataset_is_404(pool: PgPool) {
    let state = state(pool.clone());
    let (_, national) = user(&pool, Some(None)).await;

    // 999 exists nowhere; 030 exists, but only in ZNY; ZOB has no sectors at all.
    for (uri, artcc, sector_id) in [
        ("/api/v1/flow/sector-limits/ZDC/999", "ZDC", "999"),
        ("/api/v1/flow/sector-limits/ZDC/030", "ZDC", "030"),
        ("/api/v1/flow/sector-limits/ZOB/010", "ZOB", "010"),
    ] {
        let (status, _) = put(&state, uri, &national, 14).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        assert_eq!(stored(&pool, artcc, sector_id).await, None, "{uri}");
    }
}

/// A write reaches every viewer at once: the cache is reloaded in the request (a GET straight after
/// reads it, no job tick) and exactly one `flow.sector_limits` nudge goes out per write.
#[sqlx::test]
async fn a_write_reloads_the_cache_and_publishes_once(pool: PgPool) {
    let state = state(pool.clone());
    let (_, zdc) = user(&pool, Some(Some("ZDC"))).await;
    let mut rx = state.events.subscribe();

    for (limit, overridden) in [(14, true), (7, true), (10, false)] {
        let (status, _) = put(&state, ZDC_010, &zdc, limit).await;
        assert_eq!(status, StatusCode::OK, "{limit}");
        assert_eq!(drain(&mut rx), 1, "{limit}");
        assert_eq!(
            state
                .sector_limits
                .load()
                .contains_key(&("ZDC".to_string(), "010".to_string())),
            overridden,
            "{limit}"
        );
        assert_eq!(
            get(&state, "ZDC", &zdc).await["sectors"][0],
            row("010", "low", limit, overridden),
            "{limit}"
        );
    }
}
