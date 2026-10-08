//! VATUSA/OIS#723: the sector consolidation routes, through the real router — session resolution,
//! `RequirePermission`, the handler's facility scope, the same-ARTCC rule and the republish all on the
//! path.

use std::{collections::HashMap, sync::Arc};

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::broadcast::Receiver;

use crate::{
    feed::{
        sector_consolidations::SectorConsolidations,
        sector_load::{Fix, Population, Track, sector_loads},
        sectors::{SectorTable, tests::volume},
    },
    realtime::{WsEvent, topic},
    repos::sector_consolidations as repo,
    scope_test_support::{grant, seed_user, send, send_json, session_cookie, test_state},
    state::AppState,
};

const READ: &str = "flow.sectors.read";
const UPDATE: &str = "flow.sector_consolidations.update";

/// ZDC has sectors 010, 020 and 041; ZNY has 010 and 030. ZOB has none. Every volume is the same
/// fixture square, which is all the routes need.
fn state(pool: PgPool) -> AppState {
    let state = test_state(pool, HashMap::new());
    state.airspace_sectors.store(Arc::new(SectorTable {
        volumes: vec![
            volume("ZDC", "01001"),
            volume("ZDC", "02001"),
            volume("ZDC", "04101"),
            volume("ZNY", "01001"),
            volume("ZNY", "03001"),
        ],
    }));
    state
}

/// A signed-in user holding `flow.sectors.read` nationally and `update` at `update_at` (`None` for no
/// update grant, `Some(None)` for national).
async fn user(pool: &PgPool, update_at: Option<Option<&str>>) -> (String, String) {
    let id = seed_user(pool).await;
    grant(pool, &id, READ, None).await;
    if let Some(artcc) = update_at {
        grant(pool, &id, UPDATE, artcc).await;
    }
    let cookie = session_cookie(pool, &id).await;
    (id, cookie)
}

async fn request(
    state: &AppState,
    method: Method,
    uri: &str,
    cookie: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    use tower::ServiceExt;

    let mut builder = http::Request::builder()
        .method(method)
        .uri(uri)
        .header(http::header::COOKIE, cookie);
    let body = match body {
        Some(b) => {
            builder = builder.header(http::header::CONTENT_TYPE, "application/json");
            axum::body::Body::from(b.to_string())
        }
        None => axum::body::Body::empty(),
    };
    let response = crate::router::build_router(state.clone())
        .oneshot(builder.body(body).unwrap())
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

/// PUT `{artcc}/{source}` with `target` in the body.
async fn put(
    state: &AppState,
    artcc: &str,
    source: &str,
    target: &str,
    cookie: &str,
) -> (StatusCode, Value) {
    let uri = format!("/api/v1/flow/sector-consolidations/{artcc}/{source}");
    request(
        state,
        Method::PUT,
        &uri,
        cookie,
        Some(json!({ "target_sector_id": target })),
    )
    .await
}

async fn delete(state: &AppState, artcc: &str, source: &str, cookie: &str) -> (StatusCode, Value) {
    let uri = format!("/api/v1/flow/sector-consolidations/{artcc}/{source}");
    request(state, Method::DELETE, &uri, cookie, None).await
}

async fn get(state: &AppState, artcc: &str, cookie: &str) -> Value {
    let uri = format!("/api/v1/flow/sector-consolidations/{artcc}");
    let (status, body) = send_json(state, Method::GET, &uri, cookie).await;
    assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    body
}

fn pairs(rows: &[(&str, &str)]) -> Value {
    rows.iter()
        .map(|(s, t)| json!({ "sector_id": s, "target_sector_id": t }))
        .collect()
}

fn arrangement(rows: &[(&str, &str, &str)]) -> SectorConsolidations {
    rows.iter()
        .map(|(a, s, t)| ((a.to_string(), s.to_string()), t.to_string()))
        .collect()
}

/// Seed a consolidation in the DB and the cache, as another write would have left it.
async fn seed(state: &AppState, artcc: &str, source: &str, target: &str) {
    let pool = state.db.as_ref().unwrap();
    repo::consolidate(pool, artcc, source, target, None)
        .await
        .unwrap()
        .unwrap();
    state
        .sector_consolidations
        .store(Arc::new(repo::load_all(pool).await.unwrap()));
}

async fn stored(pool: &PgPool) -> SectorConsolidations {
    repo::load_all(pool).await.unwrap()
}

/// How many events arrived since the last drain, and whether all were `flow.sector_consolidations`.
fn drain(rx: &mut Receiver<WsEvent>) -> usize {
    let mut n = 0;
    while let Ok(event) = rx.try_recv() {
        assert_eq!(event.topic, topic::SECTOR_CONSOLIDATIONS);
        n += 1;
    }
    n
}

#[sqlx::test]
async fn the_list_is_one_artccs_ordered_by_sector(pool: PgPool) {
    let state = state(pool.clone());
    let (_, cookie) = user(&pool, Some(Some("ZDC"))).await;
    seed(&state, "ZDC", "041", "010").await;
    seed(&state, "ZDC", "020", "010").await;
    seed(&state, "ZNY", "030", "010").await;

    assert_eq!(
        get(&state, "zdc", &cookie).await,
        json!({
            "artcc": "ZDC",
            "editable": true,
            "consolidations": pairs(&[("020", "010"), ("041", "010")]),
        })
    );
    assert_eq!(
        get(&state, "ZOB", &cookie).await,
        json!({ "artcc": "ZOB", "editable": false, "consolidations": [] })
    );
}

#[sqlx::test]
async fn reading_needs_flow_sectors_read(pool: PgPool) {
    let state = state(pool.clone());
    let id = seed_user(&pool).await;
    grant(&pool, &id, UPDATE, None).await;
    let cookie = session_cookie(&pool, &id).await;

    let uri = "/api/v1/flow/sector-consolidations/ZDC";
    for cookie in [cookie.as_str(), ""] {
        assert_eq!(
            send(&state, Method::GET, uri, cookie, None).await,
            StatusCode::UNAUTHORIZED
        );
    }
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

/// The owner's decision: consolidation has its own permission. A reader, an anonymous caller and a
/// TMU who may only set limits are all refused, and nothing is written.
#[sqlx::test]
async fn writing_needs_flow_sector_consolidations_update(pool: PgPool) {
    let state = state(pool.clone());
    let (_, reader) = user(&pool, None).await;
    let limits_only = seed_user(&pool).await;
    grant(&pool, &limits_only, READ, None).await;
    grant(&pool, &limits_only, "flow.sector_limits.update", None).await;
    let limits_only = session_cookie(&pool, &limits_only).await;

    for cookie in [reader.as_str(), limits_only.as_str(), ""] {
        assert_eq!(
            put(&state, "ZDC", "020", "010", cookie).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            delete(&state, "ZDC", "020", cookie).await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert!(stored(&pool).await.is_empty());
}

/// A TMU consolidates their own facility's sectors, and the answer is the arrangement after the save.
#[sqlx::test]
async fn a_tmu_consolidates_their_own_facility(pool: PgPool) {
    let state = state(pool.clone());
    let (id, zdc) = user(&pool, Some(Some("ZDC"))).await;

    let (status, body) = put(&state, "zdc", "020", " 010 ", &zdc).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({ "artcc": "ZDC", "editable": true, "consolidations": pairs(&[("020", "010")]) })
    );
    assert_eq!(stored(&pool).await, arrangement(&[("ZDC", "020", "010")]));
    let by: Option<String> = sqlx::query_scalar(
        "select updated_by from flow.sector_consolidation where artcc = 'ZDC' and sector_id = '020'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(by, Some(id));
}

/// IDOR on the path: a ZDC TMU can neither consolidate nor release ZNY's sectors, upper- or
/// lower-case, and ZNY's existing consolidation survives both attempts.
#[sqlx::test]
async fn a_facility_tmu_cannot_touch_a_neighbours_sectors(pool: PgPool) {
    let state = state(pool.clone());
    let (_, zdc) = user(&pool, Some(Some("ZDC"))).await;
    seed(&state, "ZNY", "030", "010").await;
    let mut rx = state.events.subscribe();

    for artcc in ["ZNY", "zny"] {
        assert_eq!(
            put(&state, artcc, "010", "030", &zdc).await.0,
            StatusCode::FORBIDDEN,
            "{artcc}"
        );
        assert_eq!(
            put(&state, artcc, "030", "010", &zdc).await.0,
            StatusCode::FORBIDDEN,
            "{artcc}: even the stored arrangement"
        );
        assert_eq!(
            delete(&state, artcc, "030", &zdc).await.0,
            StatusCode::FORBIDDEN,
            "{artcc}"
        );
    }
    assert_eq!(stored(&pool).await, arrangement(&[("ZNY", "030", "010")]));
    assert_eq!(drain(&mut rx), 0);
}

/// AC5, and IDOR on the body: both sectors are looked up in the path's ARTCC only. A target that is
/// another ARTCC's sector (ZNY's 030) is unknown here, even to a national grant, and so is a source
/// that only exists elsewhere. Nothing is written in either ARTCC.
#[sqlx::test]
async fn a_cross_artcc_consolidation_is_refused(pool: PgPool) {
    let state = state(pool.clone());
    let (_, zdc) = user(&pool, Some(Some("ZDC"))).await;
    let (_, national) = user(&pool, Some(None)).await;
    let mut rx = state.events.subscribe();

    for cookie in [&zdc, &national] {
        for (source, target) in [("010", "030"), ("030", "010"), ("010", "999")] {
            assert_eq!(
                put(&state, "ZDC", source, target, cookie).await.0,
                StatusCode::NOT_FOUND,
                "{source} at {target}"
            );
        }
    }
    assert_eq!(
        put(&state, "ZOB", "010", "020", &national).await.0,
        StatusCode::NOT_FOUND,
        "an ARTCC with no sectors"
    );
    assert!(stored(&pool).await.is_empty());
    assert_eq!(drain(&mut rx), 0);
}

/// AC4: a self-reference is a 400 and a loop a 409. Neither writes, neither tells anyone, and the
/// arrangement already there stands.
#[sqlx::test]
async fn a_self_reference_and_a_loop_are_refused(pool: PgPool) {
    let state = state(pool.clone());
    let (_, zdc) = user(&pool, Some(Some("ZDC"))).await;
    seed(&state, "ZDC", "020", "010").await;
    let mut rx = state.events.subscribe();

    assert_eq!(
        put(&state, "ZDC", "041", "041", &zdc).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        put(&state, "ZDC", "010", "020", &zdc).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(stored(&pool).await, arrangement(&[("ZDC", "020", "010")]));
    assert_eq!(
        **state.sector_consolidations.load(),
        arrangement(&[("ZDC", "020", "010")])
    );
    assert_eq!(drain(&mut rx), 0);
}

/// AC3 through the router: 020 at 010, then 010 at 041 leaves both at 041; then 010 back onto 020
/// (a source) is saved at 041's position, never as a chain.
#[sqlx::test]
async fn a_chain_is_flattened_on_write(pool: PgPool) {
    let state = state(pool.clone());
    let (_, zdc) = user(&pool, Some(Some("ZDC"))).await;
    seed(&state, "ZDC", "020", "010").await;

    let (status, body) = put(&state, "ZDC", "010", "041", &zdc).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["consolidations"],
        pairs(&[("010", "041"), ("020", "041")])
    );

    let (status, _) = delete(&state, "ZDC", "010", &zdc).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = put(&state, "ZDC", "010", "020", &zdc).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["consolidations"],
        pairs(&[("010", "041"), ("020", "041")])
    );
    assert_eq!(
        stored(&pool).await,
        arrangement(&[("ZDC", "010", "041"), ("ZDC", "020", "041")])
    );
}

/// AC6: a consolidation reaches every viewer at once. The write reloads the cache in the request — the
/// occupancy engine, fed from the state's caches straight after, already counts the merged row — and
/// exactly one `flow.sector_consolidations` nudge goes out. The release republishes the same way. A
/// save or release that changes nothing tells nobody.
#[sqlx::test]
async fn consolidating_republishes_immediately(pool: PgPool) {
    let state = state(pool.clone());
    let (_, zdc) = user(&pool, Some(Some("ZDC"))).await;
    let mut rx = state.events.subscribe();
    let fixes = [Fix {
        t_ms: 60_000,
        lat: 38.5,
        lon: -76.5,
        alt_ft: Some(10_000.0),
    }];
    let tracks = [Track {
        id: "X",
        population: Population::Active,
        fixes: &fixes,
    }];
    // `(sector_id, consolidated, first-bin active)` of ZDC's rows, as the engine counts them now.
    let rows = |state: &AppState| -> Vec<(String, Vec<String>, usize)> {
        sector_loads(
            &state.airspace_sectors.load(),
            &state.sector_consolidations.load(),
            &tracks,
            0,
        )
        .into_iter()
        .filter(|l| l.artcc == "ZDC")
        .map(|l| (l.sector_id, l.consolidated, l.bins[0].active))
        .collect()
    };
    let own = |id: &str| (id.to_string(), Vec::new(), 1);
    assert_eq!(rows(&state), [own("010"), own("020"), own("041")]);

    assert_eq!(
        put(&state, "ZDC", "020", "010", &zdc).await.0,
        StatusCode::OK
    );
    assert_eq!(drain(&mut rx), 1);
    assert_eq!(
        **state.sector_consolidations.load(),
        arrangement(&[("ZDC", "020", "010")])
    );
    assert_eq!(
        rows(&state),
        [("010".to_string(), vec!["020".to_string()], 1), own("041")],
        "the next read already merges, without a job tick"
    );

    assert_eq!(
        put(&state, "ZDC", "020", "010", &zdc).await.0,
        StatusCode::OK
    );
    assert_eq!(drain(&mut rx), 0, "unchanged");

    let (status, body) = delete(&state, "ZDC", "020", &zdc).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["consolidations"], json!([]));
    assert_eq!(drain(&mut rx), 1);
    assert!(state.sector_consolidations.load().is_empty());
    assert_eq!(rows(&state), [own("010"), own("020"), own("041")]);

    assert_eq!(delete(&state, "ZDC", "020", &zdc).await.0, StatusCode::OK);
    assert_eq!(drain(&mut rx), 0, "nothing to release");
}

/// A national grant consolidates and releases in any ARTCC; a release needs no dataset entry, so a
/// consolidation a re-import orphaned can still be cleared.
#[sqlx::test]
async fn a_national_grant_works_any_facility(pool: PgPool) {
    let state = state(pool.clone());
    let (_, national) = user(&pool, Some(None)).await;
    seed(&state, "ZDC", "077", "010").await;

    assert_eq!(
        put(&state, "ZNY", "030", "010", &national).await.0,
        StatusCode::OK
    );
    assert_eq!(
        delete(&state, "ZDC", "077", &national).await.0,
        StatusCode::OK
    );
    assert_eq!(stored(&pool).await, arrangement(&[("ZNY", "030", "010")]));
}

/// A save or release that changes nothing still answers from the table, not this replica's cache: here
/// another replica has consolidated 020 and released 041 behind a cache that has seen neither. The
/// stale cache is put back before each call, so each no-op has to reload on its own. Neither tells
/// anyone, since nothing changed.
#[sqlx::test]
async fn a_no_op_answers_from_the_table_not_a_stale_cache(pool: PgPool) {
    let state = state(pool.clone());
    let (_, zdc) = user(&pool, Some(Some("ZDC"))).await;
    seed(&state, "ZDC", "041", "010").await;
    let stale = state.sector_consolidations.load_full();
    // Another replica's writes, which this replica's cache has not seen.
    repo::consolidate(&pool, "ZDC", "020", "010", None)
        .await
        .unwrap()
        .unwrap();
    assert!(repo::release(&pool, "ZDC", "041").await.unwrap());
    let mut rx = state.events.subscribe();

    let (status, body) = put(&state, "ZDC", "020", "010", &zdc).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["consolidations"], pairs(&[("020", "010")]), "the save");

    state.sector_consolidations.store(stale);
    let (status, body) = delete(&state, "ZDC", "041", &zdc).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["consolidations"],
        pairs(&[("020", "010")]),
        "the release"
    );
    assert_eq!(
        **state.sector_consolidations.load(),
        arrangement(&[("ZDC", "020", "010")])
    );
    assert_eq!(drain(&mut rx), 0);
}

/// The path's sector is trimmed like the body's target, so ` 020 ` names 020 on a save and a release.
#[sqlx::test]
async fn a_path_sector_is_trimmed_like_the_body(pool: PgPool) {
    let state = state(pool.clone());
    let (_, zdc) = user(&pool, Some(Some("ZDC"))).await;

    let (status, body) = put(&state, "ZDC", "%20020%20", "010", &zdc).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(stored(&pool).await, arrangement(&[("ZDC", "020", "010")]));
    assert_eq!(
        delete(&state, "ZDC", "%20020%20", &zdc).await.0,
        StatusCode::OK
    );
    assert!(stored(&pool).await.is_empty());
}

/// PUT `{artcc}` with the batch `into`.
async fn batch(state: &AppState, artcc: &str, into: Value, cookie: &str) -> (StatusCode, Value) {
    let uri = format!("/api/v1/flow/sector-consolidations/{artcc}");
    request(
        state,
        Method::PUT,
        &uri,
        cookie,
        Some(json!({ "into": into })),
    )
    .await
}

/// #794: the monitor's "All" commands are one write. Two sectors go to one target and a third is
/// released in the same request; the answer is the arrangement after it, stamped with the caller,
/// and every viewer hears about it exactly once.
#[sqlx::test]
async fn a_batch_sets_and_releases_in_one_write(pool: PgPool) {
    let state = state(pool.clone());
    let (id, zdc) = user(&pool, Some(Some("ZDC"))).await;
    seed(&state, "ZNY", "030", "010").await;
    seed(&state, "ZDC", "041", "020").await;
    let mut rx = state.events.subscribe();

    let (status, body) = batch(
        &state,
        "zdc",
        json!({ " 020 ": " 010 ", "041": null }),
        &zdc,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({ "artcc": "ZDC", "editable": true, "consolidations": pairs(&[("020", "010")]) })
    );
    assert_eq!(
        stored(&pool).await,
        arrangement(&[("ZDC", "020", "010"), ("ZNY", "030", "010")]),
        "ZNY's row is untouched"
    );
    assert_eq!(
        **state.sector_consolidations.load(),
        arrangement(&[("ZDC", "020", "010"), ("ZNY", "030", "010")])
    );
    assert_eq!(drain(&mut rx), 1, "one nudge for the whole batch");
    let by: Option<String> = sqlx::query_scalar(
        "select updated_by from flow.sector_consolidation where artcc = 'ZDC' and sector_id = '020'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(by, Some(id));

    let (status, _) = batch(&state, "ZDC", json!({ "020": "010", "041": null }), &zdc).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        drain(&mut rx),
        0,
        "a batch that changes nothing tells nobody"
    );
    let (status, body) = batch(&state, "ZDC", json!({}), &zdc).await;
    assert_eq!(
        (status, &body["consolidations"]),
        (StatusCode::OK, &pairs(&[("020", "010")]))
    );
    assert_eq!(drain(&mut rx), 0);
}

/// Consolidating a target moves its sources with it (041 at 020, then 020 at 010, leaves both at 010,
/// never 041 at 020 at 010), and a batch that releases and re-targets in one go also ends flat.
#[sqlx::test]
async fn a_batch_keeps_the_arrangement_flat(pool: PgPool) {
    let state = state(pool.clone());
    let (_, zdc) = user(&pool, Some(Some("ZDC"))).await;
    seed(&state, "ZDC", "041", "020").await;

    let (status, body) = batch(&state, "ZDC", json!({ "020": "010" }), &zdc).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["consolidations"],
        pairs(&[("020", "010"), ("041", "010")]),
        "the sources worked at 020 move with it"
    );

    // Releases land before saves: 041 gets its own row back, then 010 moves onto it and takes 020
    // with it, so 020's own entry is already true.
    let (status, body) = batch(
        &state,
        "ZDC",
        json!({ "020": "041", "010": "041", "041": null }),
        &zdc,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        stored(&pool).await,
        arrangement(&[("ZDC", "010", "041"), ("ZDC", "020", "041")])
    );
}

/// All or nothing: a refused entry anywhere in the batch writes none of it. The earlier entries (which
/// alone would save) are absent afterwards, the cache stands, and nobody is told.
#[sqlx::test]
async fn a_refused_batch_writes_nothing(pool: PgPool) {
    let state = state(pool.clone());
    let (_, zdc) = user(&pool, Some(Some("ZDC"))).await;
    seed(&state, "ZDC", "041", "010").await;
    let before = arrangement(&[("ZDC", "041", "010")]);
    let mut rx = state.events.subscribe();

    for (into, expected) in [
        (json!({ "010": "010" }), StatusCode::BAD_REQUEST),
        (
            json!({ "020": "041", "041": "041" }),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({ "020": "041", " 020": null }),
            StatusCode::BAD_REQUEST,
        ),
        // 010 at 041 resolves to 010 itself, since 041 is worked at 010: a loop.
        (json!({ "010": "041" }), StatusCode::CONFLICT),
        // A loop the batch builds between its own entries, after a save (010 at 020) that would have
        // landed on its own.
        (json!({ "010": "020", "020": "010" }), StatusCode::CONFLICT),
        (json!({ "020": "999" }), StatusCode::NOT_FOUND),
        (json!({ "020": "010", "030": "010" }), StatusCode::NOT_FOUND),
        (json!({ "020": "010", "010": "030" }), StatusCode::NOT_FOUND),
    ] {
        let (status, body) = batch(&state, "ZDC", into.clone(), &zdc).await;
        assert_eq!(status, expected, "{into}: {body}");
        assert_eq!(stored(&pool).await, before, "{into}");
    }
    assert_eq!(**state.sector_consolidations.load(), before);
    assert_eq!(drain(&mut rx), 0);
}

/// A release isn't checked against the dataset, so a batch can clear a consolidation a re-import
/// orphaned (077 is no longer one of ZDC's sectors), like the single release.
#[sqlx::test]
async fn a_batch_release_needs_no_dataset_entry(pool: PgPool) {
    let state = state(pool.clone());
    let (_, zdc) = user(&pool, Some(Some("ZDC"))).await;
    seed(&state, "ZDC", "077", "010").await;
    seed(&state, "ZDC", "020", "010").await;

    let (status, body) = batch(&state, "ZDC", json!({ "077": null, "999": null }), &zdc).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(stored(&pool).await, arrangement(&[("ZDC", "020", "010")]));
}

/// The batch is gated like the single write: the update permission, scoped to the path's ARTCC. A
/// reader, a limits-only TMU and an anonymous caller get a 401, a ZDC TMU a 403 on ZNY in either
/// case, and a national grant may write anywhere. ZNY's arrangement survives every refused attempt.
#[sqlx::test]
async fn a_batch_is_scoped_to_the_callers_facility(pool: PgPool) {
    let state = state(pool.clone());
    let (_, reader) = user(&pool, None).await;
    let (_, zdc) = user(&pool, Some(Some("ZDC"))).await;
    let (_, national) = user(&pool, Some(None)).await;
    let limits_only = seed_user(&pool).await;
    grant(&pool, &limits_only, READ, None).await;
    grant(&pool, &limits_only, "flow.sector_limits.update", None).await;
    let limits_only = session_cookie(&pool, &limits_only).await;
    seed(&state, "ZNY", "030", "010").await;
    let before = arrangement(&[("ZNY", "030", "010")]);
    let mut rx = state.events.subscribe();

    for cookie in [reader.as_str(), limits_only.as_str(), ""] {
        let (status, _) = batch(&state, "ZDC", json!({ "020": "010" }), cookie).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    for artcc in ["ZNY", "zny"] {
        for into in [json!({ "010": "030" }), json!({ "030": null })] {
            let (status, _) = batch(&state, artcc, into.clone(), &zdc).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{artcc} {into}");
        }
    }
    assert_eq!(stored(&pool).await, before);
    assert_eq!(drain(&mut rx), 0);

    let (status, body) = batch(&state, "ZNY", json!({ "030": null }), &national).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(stored(&pool).await.is_empty());
    assert_eq!(drain(&mut rx), 1);
}
