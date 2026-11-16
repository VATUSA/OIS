//! #814 AC3 and AC5: the learned taxi model is gone from the router, the contract and the source,
//! and the live Taxi Monitor it never fed still serves data.
//!
//! The migration drop is `repos::stats::tests::the_taxi_observation_table_is_dropped`; the web route
//! and nav are `web/src/router.removed-routes.test.ts` and `web/src/lib/nav.test.ts`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::http::{Method, StatusCode};
use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;
use utoipa::OpenApi;

use crate::feed::airports::Airport;
use crate::feed::taxi::TaxiSample;
use crate::feed::vatsim::{FlightPlan, Pilot, VatsimData};
use crate::scope_test_support::{grant, seed_user, send, send_json, session_cookie, test_state};

async fn holder(pool: &PgPool, permission: &str) -> String {
    let user = seed_user(pool).await;
    grant(pool, &user, permission, None).await;
    session_cookie(pool, &user).await
}

#[sqlx::test]
async fn the_removed_taxi_insight_routes_answer_404_to_a_stats_reader(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let cookie = holder(&pool, "stats.data.read").await;
    for uri in [
        "/api/v1/stats/taxi/observations",
        "/api/v1/stats/taxi/estimates",
    ] {
        for who in [cookie.as_str(), ""] {
            let status = send(&state, Method::GET, uri, who, None).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri} as {who:?}");
        }
    }
    let status = send(
        &state,
        Method::GET,
        "/api/v1/stats/hist/taxi/KJFK?at=1700000000",
        &cookie,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "positive control: the reader reaches a sibling stats route"
    );
}

#[test]
fn the_contract_no_longer_describes_the_learned_taxi_model() {
    let doc = serde_json::to_value(crate::openapi::ApiDoc::openapi()).unwrap();
    let paths = doc["paths"].as_object().unwrap();
    for gone in [
        "/api/v1/stats/taxi/observations",
        "/api/v1/stats/taxi/estimates",
    ] {
        assert!(!paths.contains_key(gone), "{gone} is still documented");
    }
    for kept in ["/api/v1/stats/hist/taxi/{icao}", "/api/v1/tmu/taxi/{icao}"] {
        assert!(paths.contains_key(kept), "positive control: {kept}");
    }
    let schemas = doc["components"]["schemas"].as_object().unwrap();
    let stale: Vec<&String> = schemas
        .keys()
        .filter(|k| k.contains("TaxiObservation") || k.contains("TaxiEstimate"))
        .collect();
    assert!(stale.is_empty(), "schemas still present: {stale:?}");
    let debug = schemas["FcaFlightDebug"]["properties"].as_object().unwrap();
    assert!(debug.contains_key("profile"), "positive control: {debug:?}");
    assert!(!debug.contains_key("taxi_estimate"), "{debug:?}");
}

/// Names only the removed model carried. `taxi_observation` (singular) stays legal: the migrations
/// that created and dropped the table name it.
const REMOVED: [&str; 8] = [
    "taxi_observations",
    "taxi_estimate",
    "taxi_insights",
    "taxi-insights",
    "TaxiInsights",
    "resolve_ground_allowance",
    "ground_allowance",
    "/api/v1/stats/taxi/",
];

/// Files a reference may live in, each for a reason: the migrations (the ones that created and
/// dropped the table among them; applied migrations are never edited), the changelog entry that
/// records the removal, and this file.
const EXEMPT: [&str; 3] = [
    "backend/migrations/",
    "docs-site/reference/api-changelog.md",
    "backend/src/handlers/taxi_tests.rs",
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .into()
}

fn text_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if !matches!(name.as_str(), "node_modules" | "target" | "dist" | "cache") {
                text_files(&path, out);
            }
        } else if [".rs", ".ts", ".tsx", ".md", ".mts", ".sql"]
            .iter()
            .any(|ext| name.ends_with(ext))
        {
            out.push(path);
        }
    }
}

/// `path:line: token` for every removed name under `root`'s `dirs`, skipping [`EXEMPT`]; and how many
/// files were read.
fn removed_names(root: &Path, dirs: &[&str]) -> (Vec<String>, usize) {
    let mut files = Vec::new();
    for d in dirs {
        text_files(&root.join(d), &mut files);
    }
    let mut hits = Vec::new();
    for file in &files {
        let rel = file
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if EXEMPT.iter().any(|e| rel.starts_with(e)) {
            continue;
        }
        let text = std::fs::read_to_string(file).unwrap_or_else(|e| panic!("{rel}: {e}"));
        for (i, line) in text.lines().enumerate() {
            for token in REMOVED.iter().filter(|t| line.contains(**t)) {
                hits.push(format!("{rel}:{}: {token}", i + 1));
            }
        }
    }
    (hits, files.len())
}

#[test]
fn no_source_or_doc_names_the_removed_taxi_model() {
    let dirs = [
        "backend/src",
        "crates",
        "discord/src",
        "web/src",
        "packages",
        "docs",
        "docs-site",
    ];
    let (hits, scanned) = removed_names(&repo_root(), &dirs);
    assert!(scanned > 500, "the walk is live: {scanned} files");
    assert!(
        hits.is_empty(),
        "the removed taxi model is still named:\n{hits:#?}"
    );
}

#[test]
fn the_removed_taxi_model_scan_catches_a_planted_reference() {
    let root = std::env::temp_dir().join(format!("ois-814-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.rs"), "use crate::feed::taxi_estimate;\n").unwrap();
    std::fs::write(
        root.join("src/b.md"),
        "the TaxiMonitor page and stats.taxi_observation\n",
    )
    .unwrap();
    let (hits, scanned) = removed_names(&root, &["src"]);
    std::fs::remove_dir_all(&root).unwrap();
    assert_eq!(scanned, 2);
    assert_eq!(hits, vec!["src/a.rs:1: taxi_estimate".to_string()]);
}

/// Every name the scan guards is live: one planted per file, each caught in its own.
#[test]
fn the_removed_taxi_model_scan_catches_every_name_it_guards() {
    let root = std::env::temp_dir().join(format!("ois-814-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(root.join("src")).unwrap();
    for (n, token) in REMOVED.iter().enumerate() {
        std::fs::write(root.join(format!("src/t{n}.rs")), format!("x {token} y\n")).unwrap();
    }
    let (hits, scanned) = removed_names(&root, &["src"]);
    std::fs::remove_dir_all(&root).unwrap();
    assert_eq!(scanned, REMOVED.len());
    for (n, token) in REMOVED.iter().enumerate() {
        let want = format!("src/t{n}.rs:1: {token}");
        assert!(hits.contains(&want), "{token} is not caught: {hits:?}");
    }
}

fn kjfk() -> crate::feed::airports::AirportDb {
    HashMap::from([("KJFK".to_string(), Airport::at(40.64, -73.78))])
}

fn gnd1(gs: i64, alt: i64) -> Pilot {
    Pilot {
        callsign: "GND1".into(),
        latitude: 40.64,
        longitude: -73.78,
        altitude: alt,
        groundspeed: gs,
        flight_plan: Some(FlightPlan {
            departure: "KJFK".into(),
            arrival: "KDCA".into(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[sqlx::test]
async fn the_live_taxi_monitor_refuses_a_caller_without_program_read(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let uri = "/api/v1/tmu/taxi/KJFK";
    assert_eq!(
        send(&state, Method::GET, uri, "", None).await,
        StatusCode::UNAUTHORIZED
    );
    let stats_only = holder(&pool, "stats.data.read").await;
    assert_eq!(
        send(&state, Method::GET, uri, &stats_only, None).await,
        StatusCode::UNAUTHORIZED
    );
    let reader = holder(&pool, "tmu.program.read").await;
    let (status, body) = send_json(&state, Method::GET, uri, &reader).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["icao"], "KJFK");
}

#[sqlx::test]
async fn the_live_taxi_monitor_serves_the_fields_samples_and_active_rolls(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let now = Utc::now();
    let ms = |min: i64| (now - Duration::minutes(min)).timestamp_millis();
    {
        let mut feed = state.feed.write().await;
        feed.airports = Arc::new(kjfk());
        feed.taxi_samples.insert(
            "KJFK".into(),
            vec![
                TaxiSample {
                    end_ms: ms(5),
                    duration_ms: 600_000,
                },
                TaxiSample {
                    end_ms: ms(20),
                    duration_ms: 900_000,
                },
            ],
        );
        // Two ticks through the monitor's own state machine: parked, then taxiing.
        let airports = feed.airports.clone();
        let parked = VatsimData {
            pilots: vec![gnd1(0, 13)],
            ..Default::default()
        };
        let taxiing = VatsimData {
            pilots: vec![gnd1(15, 13)],
            ..Default::default()
        };
        let inner = &mut *feed;
        crate::feed::taxi::process(
            &mut inner.taxi_sessions,
            &mut inner.taxi_samples,
            &airports,
            &parked,
            now - Duration::seconds(30),
        );
        crate::feed::taxi::process(
            &mut inner.taxi_sessions,
            &mut inner.taxi_samples,
            &airports,
            &taxiing,
            now,
        );
        inner.snapshot = Some(Arc::new(crate::feed::Snapshot::of(taxiing)));
    }
    let reader = holder(&pool, "tmu.program.read").await;
    let (status, body) = send_json(&state, Method::GET, "/api/v1/tmu/taxi/KJFK", &reader).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["icao"], "KJFK");
    assert_eq!(body["sample_count"], 2, "{body}");
    // (600 + 900) / 2 s = 12.5 min, rounded.
    assert_eq!(body["avg_min"], 13, "{body}");
    assert_eq!(body["active"][0]["callsign"], "GND1", "{body}");
    assert_eq!(body["active"][0]["phase"], "rolling", "{body}");
}

#[sqlx::test]
async fn the_historical_taxi_monitor_refuses_a_caller_without_stats_read(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let uri = "/api/v1/stats/hist/taxi/KJFK?at=1700000000";
    assert_eq!(
        send(&state, Method::GET, uri, "", None).await,
        StatusCode::UNAUTHORIZED
    );
    let program_only = holder(&pool, "tmu.program.read").await;
    assert_eq!(
        send(&state, Method::GET, uri, &program_only, None).await,
        StatusCode::UNAUTHORIZED
    );
    let reader = holder(&pool, "stats.data.read").await;
    assert_eq!(
        send(&state, Method::GET, uri, &reader, None).await,
        StatusCode::OK
    );
    assert_eq!(
        send(
            &state,
            Method::GET,
            "/api/v1/stats/hist/taxi/KJFK",
            &reader,
            None
        )
        .await,
        StatusCode::BAD_REQUEST,
        "no instant"
    );
}

#[sqlx::test]
async fn the_historical_taxi_monitor_replays_a_departure_roll_at_t(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    state.feed.write().await.airports = Arc::new(kjfk());
    let at: DateTime<Utc> = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
    sqlx::query(
        "insert into stats.flight (session_id, cid, callsign, logon_time, first_seen, last_seen, \
             status, departure, arrival) \
         values (1, 1000001, 'GND1', $1, $1, $2, 'active', 'KJFK', 'KDCA')",
    )
    .bind(at - Duration::hours(1))
    .bind(at)
    .execute(&pool)
    .await
    .unwrap();
    // Parked, taxiing 30 s later, airborne 90 s after that: one 90 s roll, ending 20 min before T.
    let start = at - Duration::minutes(22);
    for (secs, gs, alt) in [(0, 0, 13), (30, 20, 13), (120, 80, 400)] {
        sqlx::query(
            "insert into stats.position (session_id, ts, lat, lon, altitude, groundspeed, heading) \
             values (1, $1, 40.64, -73.78, $2, $3, 40)",
        )
        .bind(start + Duration::seconds(secs))
        .bind(alt)
        .bind(gs)
        .execute(&pool)
        .await
        .unwrap();
    }
    let reader = holder(&pool, "stats.data.read").await;
    let uri = |t: DateTime<Utc>| format!("/api/v1/stats/hist/taxi/KJFK?at={}", t.timestamp());
    let (status, body) = send_json(&state, Method::GET, &uri(at), &reader).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["sample_count"], 1, "{body}");
    assert_eq!(body["last_sec"], 90, "{body}");
    let (_, before) = send_json(
        &state,
        Method::GET,
        &uri(start - Duration::minutes(1)),
        &reader,
    )
    .await;
    assert_eq!(
        before["sample_count"], 0,
        "nothing had rolled yet: {before}"
    );
}
