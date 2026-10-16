//! A prefile is not an FCA candidate (#834). Every surface built on `build_candidates` (the IDST
//! board, FCA traffic, metering, the flight advisory, release marking) and the `/flow/counts` badge
//! read connected pilots with a flight plan only, so an aircraft in both `pilots` and `prefiles` is
//! listed once, and a prefile alone is neither listed nor metered.
//!
//! Every test goes through the real router. The feed mirrors `machine_actor_tests::crossing_state`:
//! KJFK->KDCA via `RBV WHITE SIE`, crossing an FCA on the JFK->DCA corridor between WHITE and SIE.

use std::collections::HashMap;
use std::sync::Arc;

use axum::http::{self, Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::feed::airports::Airport;
use crate::feed::vatsim::{FlightPlan, Pilot, Prefile, VatsimData};
use crate::scope_test_support::{grant, seed_user, send_json, session_cookie, test_state};
use crate::state::AppState;

fn plan(dep: &str) -> FlightPlan {
    FlightPlan {
        departure: dep.into(),
        arrival: "KDCA".into(),
        route: "RBV WHITE SIE".into(),
        altitude: "35000".into(),
        cruise_tas: "440".into(),
        ..Default::default()
    }
}

/// A connected pilot parked at KJFK.
fn parked_at_jfk(callsign: &str) -> Pilot {
    Pilot {
        callsign: callsign.into(),
        latitude: 40.64,
        longitude: -73.78,
        altitude: 13,
        groundspeed: 0,
        heading: 310,
        flight_plan: Some(plan("KJFK")),
        ..Default::default()
    }
}

fn prefile(callsign: &str, dep: &str) -> Prefile {
    Prefile {
        callsign: callsign.into(),
        flight_plan: Some(plan(dep)),
        ..Default::default()
    }
}

async fn install_feed(state: &AppState, data: VatsimData) {
    let mut feed = state.feed.write().await;
    feed.airports = Arc::new(HashMap::from([
        ("KJFK".to_string(), Airport::at(40.64, -73.78)),
        ("KEWR".to_string(), Airport::at(40.69, -74.17)),
        ("KDCA".to_string(), Airport::at(38.85, -77.04)),
    ]));
    feed.snapshot = Some(Arc::new(crate::feed::Snapshot::of(data)));
}

/// GND1 connected and still prefiled, PRE2 prefiled only.
fn pilot_with_lingering_prefile_and_a_prefile_only() -> VatsimData {
    VatsimData {
        pilots: vec![parked_at_jfk("GND1")],
        prefiles: vec![prefile("GND1", "KJFK"), prefile("PRE2", "KJFK")],
        ..Default::default()
    }
}

/// An enabled rate FCA on the corridor, and its id.
async fn corridor_fca(pool: &PgPool, name: &str, rate: i32) -> String {
    sqlx::query_scalar(
        "insert into flow.fca (name, color, artcc, points, dests, origins, fixes, scope, \
             dir, mode, rate, mit, enabled) \
         values ($1, '#fff', 'ZDC', $2, '{}', '{}', '{}', '{}', 'any', 'rate', $3, 0, true) \
         returning id",
    )
    .bind(name)
    .bind(json!([[39.5, -75.6], [39.5, -74.0]]))
    .bind(rate)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// A national holder of `permission`, as a session cookie.
async fn holder(pool: &PgPool, permission: &str) -> String {
    let user = seed_user(pool).await;
    grant(pool, &user, permission, None).await;
    session_cookie(pool, &user).await
}

/// One request with a JSON body through the real router; `send_json` sends no body.
async fn call(
    state: &AppState,
    method: Method,
    uri: &str,
    cookie: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    use tower::ServiceExt;

    let builder = http::Request::builder()
        .method(method)
        .uri(uri)
        .header(http::header::COOKIE, cookie);
    let request = match body {
        Some(b) => builder
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(b.to_string())),
        None => builder.body(axum::body::Body::empty()),
    }
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

async fn release_count(pool: &PgPool, fca_id: &str, callsign: &str) -> i64 {
    sqlx::query_scalar("select count(*) from flow.fca_release where fca_id = $1 and callsign = $2")
        .bind(fca_id)
        .bind(callsign)
        .fetch_one(pool)
        .await
        .unwrap()
}

fn callsigns_and_statuses(rows: &Value) -> Vec<(String, String)> {
    rows.as_array()
        .expect("an array of flights")
        .iter()
        .map(|f| {
            (
                f["callsign"].as_str().unwrap().to_string(),
                f["status"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

/// AC4: the badge counts what the board lists. Before #834 this FCA counted 3: the pilot, its own
/// prefile and PRE2.
#[sqlx::test]
async fn flow_counts_counts_a_connected_pilot_once_and_no_prefile(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    install_feed(&state, pilot_with_lingering_prefile_and_a_prefile_only()).await;
    let fca_id = corridor_fca(&pool, "T834", 30).await;

    // Public by design (#586), so no credential.
    let (status, body) = send_json(&state, Method::GET, "/api/v1/flow/counts", "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body[&fca_id], 1, "{body}");
}

/// AC4: an FCA crossed only by prefiles counts zero, and keeps its key. The control adds a pilot.
#[sqlx::test]
async fn flow_counts_is_zero_for_an_fca_only_prefiles_cross(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    install_feed(
        &state,
        VatsimData {
            prefiles: vec![prefile("PRE1", "KJFK"), prefile("PRE2", "KJFK")],
            ..Default::default()
        },
    )
    .await;
    let fca_id = corridor_fca(&pool, "T834", 30).await;

    let (status, body) = send_json(&state, Method::GET, "/api/v1/flow/counts", "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body[&fca_id], 0, "{body}");

    install_feed(
        &state,
        VatsimData {
            pilots: vec![parked_at_jfk("GND1")],
            prefiles: vec![prefile("PRE1", "KJFK"), prefile("PRE2", "KJFK")],
            ..Default::default()
        },
    )
    .await;
    let (_, body) = send_json(&state, Method::GET, "/api/v1/flow/counts", "").await;
    assert_eq!(
        body[&fca_id], 1,
        "control: a connected pilot counts: {body}"
    );
}

/// Climbing out of KJFK, short of the corridor line, on the same plan: it crosses the FCA airborne.
fn airborne_out_of_jfk(callsign: &str) -> Pilot {
    Pilot {
        callsign: callsign.into(),
        latitude: 40.45,
        longitude: -74.1,
        altitude: 8_000,
        groundspeed: 300,
        heading: 230,
        flight_plan: Some(plan("KJFK")),
        ..Default::default()
    }
}

async fn seed_release(pool: &PgPool, fca_id: &str, callsign: &str) {
    let cta = chrono::Utc::now().timestamp_millis() + 3_600_000;
    sqlx::query(
        "insert into flow.fca_release (fca_id, callsign, cta_ms, edct_ms) values ($1, $2, $3, $3)",
    )
    .bind(fca_id)
    .bind(callsign)
    .bind(cta)
    .execute(pool)
    .await
    .unwrap();
}

/// AC1: IDST and the FCA traffic route list connected pilots only. IDST lists ground departures at
/// the airports in scope, so the airborne KJFK departure and the ground pilot at KEWR are on the
/// traffic route (the control) and not on IDST for KJFK. PRE2's stored release stays out of
/// `released` until PRE2 connects.
#[sqlx::test]
async fn idst_and_fca_traffic_list_a_ground_pilot_once_and_no_prefile(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let mut data = pilot_with_lingering_prefile_and_a_prefile_only();
    data.pilots.push(airborne_out_of_jfk("AIR1"));
    let mut at_newark = parked_at_jfk("EWR1");
    (at_newark.latitude, at_newark.longitude) = (40.69, -74.17);
    at_newark.flight_plan = Some(plan("KEWR"));
    data.pilots.push(at_newark);
    install_feed(&state, data).await;
    let fca_id = corridor_fca(&pool, "T834", 30).await;
    seed_release(&pool, &fca_id, "PRE2").await;
    let reader = holder(&pool, "flow.fca.read").await;
    let idst_uri = "/api/v1/flow/idst?airports=KJFK";

    let (status, idst) = send_json(&state, Method::GET, idst_uri, &reader).await;
    assert_eq!(status, StatusCode::OK, "{idst}");
    assert_eq!(
        callsigns_and_statuses(&idst["unscheduled"]),
        vec![("GND1".to_string(), "ground".to_string())],
        "{idst}"
    );
    assert_eq!(
        idst["released"],
        json!([]),
        "PRE2 has not connected: {idst}"
    );
    assert_eq!(idst["unscheduled"][0]["fca_id"], fca_id.as_str());
    assert_eq!(idst["metered_count"], 1);

    let (status, traffic) = send_json(
        &state,
        Method::GET,
        &format!("/api/v1/flow/fcas/{fca_id}/traffic"),
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{traffic}");
    let mut rows = callsigns_and_statuses(&traffic);
    rows.sort();
    assert_eq!(
        rows,
        vec![
            ("AIR1".to_string(), "airborne".to_string()),
            ("EWR1".to_string(), "ground".to_string()),
            ("GND1".to_string(), "ground".to_string()),
        ],
        "{traffic}"
    );
    let (_, newark) = send_json(
        &state,
        Method::GET,
        "/api/v1/flow/idst?airports=KEWR",
        &reader,
    )
    .await;
    assert_eq!(
        callsigns_and_statuses(&newark["unscheduled"]),
        vec![("EWR1".to_string(), "ground".to_string())],
        "control: EWR1 is listed for its own airport: {newark}"
    );

    let mut connected = pilot_with_lingering_prefile_and_a_prefile_only();
    connected.pilots.push(parked_at_jfk("PRE2"));
    install_feed(&state, connected).await;
    let (_, idst) = send_json(&state, Method::GET, idst_uri, &reader).await;
    assert_eq!(
        callsigns_and_statuses(&idst["released"]),
        vec![("PRE2".to_string(), "ground".to_string())],
        "once connected, PRE2's stored release applies: {idst}"
    );
}

/// AC1, `mark_release`: RDY on a flight that has not connected is 404 and writes nothing. The same
/// request for the connected pilot is the control.
#[sqlx::test]
async fn marking_a_release_for_a_prefile_only_callsign_answers_404(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    install_feed(&state, pilot_with_lingering_prefile_and_a_prefile_only()).await;
    let fca_id = corridor_fca(&pool, "T834", 30).await;
    let writer = holder(&pool, "flow.fca.update").await;

    let (status, body) = call(
        &state,
        Method::POST,
        &format!("/api/v1/flow/fcas/{fca_id}/release/PRE2"),
        &writer,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(release_count(&pool, &fca_id, "PRE2").await, 0);

    let (status, body) = call(
        &state,
        Method::POST,
        &format!("/api/v1/flow/fcas/{fca_id}/release/GND1"),
        &writer,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "control: {body}");
    assert_eq!(release_count(&pool, &fca_id, "GND1").await, 1);
}

/// AC1, `clear_release`: a release stored for a prefile-only callsign still clears, and the
/// re-metered list it returns holds the connected pilot only.
#[sqlx::test]
async fn the_clear_release_response_lists_no_prefile(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    install_feed(&state, pilot_with_lingering_prefile_and_a_prefile_only()).await;
    let fca_id = corridor_fca(&pool, "T834", 30).await;
    let writer = holder(&pool, "flow.fca.update").await;
    seed_release(&pool, &fca_id, "PRE2").await;

    let (status, body) = call(
        &state,
        Method::DELETE,
        &format!("/api/v1/flow/fcas/{fca_id}/release/PRE2"),
        &writer,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(release_count(&pool, &fca_id, "PRE2").await, 0);
    assert_eq!(
        callsigns_and_statuses(&body),
        vec![("GND1".to_string(), "ground".to_string())],
        "{body}"
    );
}

/// AC1, the flight advisory (`metered_flights`): a prefile departing KEWR crosses ahead of the pilot
/// at KJFK, and before #834 the pilot owed it 5 min on a 10/hr FCA. Now the pilot is alone.
#[sqlx::test]
async fn the_flight_advisory_meters_a_pilot_without_a_prefile_ahead(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    install_feed(
        &state,
        VatsimData {
            pilots: vec![parked_at_jfk("GND1")],
            prefiles: vec![prefile("PRE1", "KEWR")],
            ..Default::default()
        },
    )
    .await;
    let fca_id = corridor_fca(&pool, "T834", 10).await;

    let (status, body) = send_json(&state, Method::GET, "/api/v1/public/flight/GND1", "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["found"], true, "{body}");
    let fcas = body["fcas"].as_array().expect("fcas");
    assert_eq!(fcas.len(), 1, "{body}");
    assert_eq!(fcas[0]["fca_id"], fca_id.as_str());
    assert_eq!(fcas[0]["seq"], 1, "{body}");
    assert_eq!(fcas[0]["delay_min"], 0, "{body}");
}

/// AC3: path 2 is unchanged here (#862): a pilot crossing two FCAs has a row in each.
#[sqlx::test]
async fn a_pilot_crossing_two_fcas_still_has_one_idst_row_per_fca(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    install_feed(
        &state,
        VatsimData {
            pilots: vec![parked_at_jfk("GND1")],
            ..Default::default()
        },
    )
    .await;
    let a = corridor_fca(&pool, "T834A", 30).await;
    let b = corridor_fca(&pool, "T834B", 30).await;
    let reader = holder(&pool, "flow.fca.read").await;

    let (status, idst) = send_json(
        &state,
        Method::GET,
        "/api/v1/flow/idst?airports=KJFK",
        &reader,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{idst}");
    let mut fcas: Vec<&str> = idst["unscheduled"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            assert_eq!(f["callsign"], "GND1");
            f["fca_id"].as_str().unwrap()
        })
        .collect();
    fcas.sort_unstable();
    let mut expected = vec![a.as_str(), b.as_str()];
    expected.sort_unstable();
    assert_eq!(fcas, expected, "{idst}");
}
