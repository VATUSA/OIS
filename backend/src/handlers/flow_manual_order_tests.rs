//! A manual FCA order through the real routes (#819): `PUT /api/v1/flow/fcas/{id}/order`, then the
//! sequence `GET /api/v1/flow/fcas/{id}/traffic` and `GET /api/v1/flow/idst` read back. A flight the
//! order does not name is merged in by ETA, a stored callsign matches in any case, and every flight
//! carries its gap to the flight ahead.
//!
//! The feed mirrors `flow_prefile_candidate_tests`: KJFK->KDCA via `RBV WHITE SIE`, crossing an FCA
//! on the JFK->DCA corridor.

use std::collections::HashMap;
use std::sync::Arc;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::feed::airports::Airport;
use crate::feed::vatsim::{FlightPlan, Pilot, VatsimData};
use crate::scope_test_support::{grant, seed_user, send, send_json, session_cookie, test_state};
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

/// Airborne out of KJFK at 8,000 ft on the way to RBV, `frac` of the way there: the larger `frac`,
/// the nearer the line and the earlier the ETA.
fn airborne(callsign: &str, frac: f64) -> Pilot {
    let (lat, lon) = (
        40.64 + (40.20 - 40.64) * frac,
        -73.78 + (-74.49 + 73.78) * frac,
    );
    Pilot {
        callsign: callsign.into(),
        latitude: lat,
        longitude: lon,
        altitude: 8_000,
        groundspeed: 300,
        heading: 230,
        flight_plan: Some(plan("KJFK")),
        ..Default::default()
    }
}

async fn install_feed(state: &AppState, pilots: Vec<Pilot>) {
    let mut feed = state.feed.write().await;
    feed.airports = Arc::new(HashMap::from([
        ("KJFK".to_string(), Airport::at(40.64, -73.78)),
        ("KBOS".to_string(), Airport::at(42.36, -71.01)),
        ("KDCA".to_string(), Airport::at(38.85, -77.04)),
    ]));
    feed.snapshot = Some(Arc::new(crate::feed::Snapshot::of(VatsimData {
        pilots,
        ..Default::default()
    })));
}

/// An enabled rate-30 FCA on the corridor, and its id.
async fn corridor_fca(pool: &PgPool) -> String {
    sqlx::query_scalar(
        "insert into flow.fca (name, color, artcc, points, dests, origins, fixes, scope, \
             dir, mode, rate, mit, enabled) \
         values ('T819', '#fff', 'ZDC', $1, '{}', '{}', '{}', '{}', 'any', 'rate', 30, 0, true) \
         returning id",
    )
    .bind(json!([[39.5, -75.6], [39.5, -74.0]]))
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn holder(pool: &PgPool, permission: &str) -> String {
    let user = seed_user(pool).await;
    grant(pool, &user, permission, None).await;
    session_cookie(pool, &user).await
}

async fn traffic(state: &AppState, id: &str) -> Vec<Value> {
    let (status, body) = send_json(
        state,
        Method::GET,
        &format!("/api/v1/flow/fcas/{id}/traffic"),
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut rows = body.as_array().expect("an array of flights").clone();
    rows.sort_by_key(|f| f["seq"].as_i64());
    rows
}

fn field<'a>(rows: &'a [Value], key: &str) -> Vec<&'a Value> {
    rows.iter().map(|f| &f[key]).collect()
}

/// AC2: two aircraft at the same point have the same ETA, so the second is pushed back by the rate
/// interval (120 s), and its gap is the spacing enforced. The leader has no gap.
#[sqlx::test]
async fn the_traffic_response_carries_gap_to_leader_and_required_spacing(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    install_feed(&state, vec![airborne("AAL1", 0.5), airborne("DAL2", 0.5)]).await;
    let id = corridor_fca(&pool).await;

    let rows = traffic(&state, &id).await;
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[0]["gap_to_leader_sec"], Value::Null);
    assert_eq!(rows[0]["gap_to_leader_nm"], Value::Null);
    let follower = &rows[1];
    assert!(follower["delay_sec"].as_i64().unwrap() > 0, "{follower}");
    assert_eq!(follower["required_spacing_sec"], 120);
    assert_eq!(
        follower["gap_to_leader_sec"],
        follower["required_spacing_sec"]
    );
    assert_eq!(
        follower["gap_to_leader_nm"],
        follower["required_spacing_nm"]
    );
    assert!(follower["cross_speed"].as_i64().unwrap() > 0, "{follower}");
    assert_eq!(follower["manually_placed"], false, "auto places nothing");
}

/// AC1 + AC4: an order saved in lowercase names AAL1 and DAL2; JOIN9 arrives between them by ETA and
/// is merged there, not at the tail. With exact-case matching nothing would be manually placed.
#[sqlx::test]
async fn a_saved_order_in_another_case_merges_a_late_joiner_in_the_traffic_response(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    install_feed(
        &state,
        vec![
            airborne("AAL1", 0.8),
            airborne("JOIN9", 0.5),
            airborne("DAL2", 0.2),
        ],
    )
    .await;
    let id = corridor_fca(&pool).await;
    let editor = holder(&pool, "flow.fca.update").await;

    assert_eq!(
        send(
            &state,
            Method::PUT,
            &format!("/api/v1/flow/fcas/{id}/order"),
            &editor,
            Some(json!({ "order": ["aal1", "dal2"] })),
        )
        .await,
        StatusCode::NO_CONTENT
    );
    let rows = traffic(&state, &id).await;
    assert_eq!(
        field(&rows, "callsign"),
        [&json!("AAL1"), &json!("JOIN9"), &json!("DAL2")],
        "{rows:?}"
    );
    assert_eq!(
        field(&rows, "manually_placed"),
        [&json!(true), &json!(false), &json!(true)]
    );
}

/// NEAR1 airborne near the line, GND1 parked at KJFK, FAR1 at cruise out by Boston and FARX further
/// out still: by ETA, NEAR1, GND1, FAR1, FARX.
fn near_ground_far() -> Vec<Pilot> {
    let near = airborne("NEAR1", 0.8);
    let ground = Pilot {
        callsign: "GND1".into(),
        latitude: 40.64,
        longitude: -73.78,
        altitude: 13,
        groundspeed: 0,
        heading: 310,
        flight_plan: Some(plan("KJFK")),
        ..Default::default()
    };
    let far = Pilot {
        callsign: "FAR1".into(),
        latitude: 42.2,
        longitude: -71.5,
        altitude: 35_000,
        groundspeed: 440,
        heading: 230,
        flight_plan: Some(plan("KBOS")),
        ..Default::default()
    };
    let farther = Pilot {
        callsign: "FARX".into(),
        latitude: 42.9,
        longitude: -70.8,
        ..far.clone()
    };
    vec![near, ground, far, farther]
}

/// FARX dragged to the front of an order that also names NEAR1 and FAR1, so the three ways GND1 can
/// be sequenced differ: by ETA (the order ignored) it is 2nd, merged it is 3rd (behind NEAR1, the
/// last ordered flight with an earlier ETA), and appended at the tail it is 4th.
const ORDER: [&str; 3] = ["FARX", "NEAR1", "FAR1"];

/// Saves `order` as the FCA's manual sequence through the real route.
async fn save_order(state: &AppState, id: &str, editor: &str, order: &[&str]) {
    assert_eq!(
        send(
            state,
            Method::PUT,
            &format!("/api/v1/flow/fcas/{id}/order"),
            editor,
            Some(json!({ "order": order })),
        )
        .await,
        StatusCode::NO_CONTENT
    );
}

/// AC1 at the IDST caller: a ground departure at KJFK that the order does not name is sequenced
/// at its merged slot, 3rd under [`ORDER`].
#[sqlx::test]
async fn the_idst_list_sequences_a_late_joiner_at_its_merged_slot(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    install_feed(&state, near_ground_far()).await;
    let id = corridor_fca(&pool).await;
    let editor = holder(&pool, "flow.fca.update").await;
    let reader = holder(&pool, "flow.fca.read").await;

    // Auto first: the fixture's own ETA order.
    let rows = traffic(&state, &id).await;
    assert_eq!(
        field(&rows, "callsign"),
        [
            &json!("NEAR1"),
            &json!("GND1"),
            &json!("FAR1"),
            &json!("FARX")
        ],
        "fixture: {rows:?}"
    );
    save_order(&state, &id, &editor, &ORDER).await;

    let (status, idst) = send_json(
        &state,
        Method::GET,
        "/api/v1/flow/idst?airports=KJFK",
        &reader,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{idst}");
    let row = &idst["unscheduled"][0];
    assert_eq!(row["callsign"], "GND1", "{idst}");
    assert_eq!(
        row["seq"], 3,
        "merged behind NEAR1, not at the tail: {idst}"
    );
}

/// [`send`] with a JSON body, returning the decoded response body as well.
async fn send_body(
    state: &AppState,
    method: Method,
    uri: &str,
    cookie: &str,
    body: Value,
) -> (StatusCode, Value) {
    use tower::ServiceExt;

    let request = axum::http::Request::builder()
        .method(method)
        .uri(uri)
        .header(axum::http::header::COOKIE, cookie)
        .header(axum::http::header::CONTENT_TYPE, "application/json")
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

/// AC1 at the release caller: RDY on a late joiner in a manual FCA releases it into its merged slot
/// (3rd under [`ORDER`], behind NEAR1 and ahead of FAR1) plus the RDY margin, so its gap to NEAR1
/// grows by exactly `RELEASE_MARGIN_MS`. Each gap comes from one response, and every ETA in it moves
/// with that request's clock, so time passing between the two requests cannot shift the comparison.
#[sqlx::test]
async fn rdy_on_a_late_joiner_releases_it_into_its_merged_slot(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    install_feed(&state, near_ground_far()).await;
    let id = corridor_fca(&pool).await;
    let editor = holder(&pool, "flow.fca.update").await;
    save_order(&state, &id, &editor, &ORDER).await;

    let before = traffic(&state, &id).await;
    assert_eq!(
        field(&before, "callsign"),
        [
            &json!("FARX"),
            &json!("NEAR1"),
            &json!("GND1"),
            &json!("FAR1")
        ],
        "merged before the release: {before:?}"
    );
    let gap = before[2]["gap_to_leader_sec"]
        .as_i64()
        .expect("GND1 has a flight ahead");

    let (status, body) = send_body(
        &state,
        Method::POST,
        &format!("/api/v1/flow/fcas/{id}/release/GND1"),
        &editor,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let after = body.as_array().expect("the re-metered flights").clone();
    assert_eq!(
        field(&after, "callsign"),
        [
            &json!("FARX"),
            &json!("NEAR1"),
            &json!("GND1"),
            &json!("FAR1")
        ],
        "{after:?}"
    );
    assert_eq!(after[2]["released"], true, "{after:?}");
    assert_eq!(
        after[2]["gap_to_leader_sec"].as_i64(),
        Some(gap + 15),
        "released at its merged slot plus RELEASE_MARGIN_MS: {after:?}"
    );
}

/// AC1 at the flight advisory (`GET /api/v1/public/flight/{callsign}`, which `GET /api/v1/me/flight`
/// builds through the same `build_flight_advisory`): the late joiner is 3rd under [`ORDER`], not 2nd
/// by ETA or 4th at the tail.
#[sqlx::test]
async fn the_flight_advisory_sequences_a_late_joiner_at_its_merged_slot(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    install_feed(&state, near_ground_far()).await;
    let id = corridor_fca(&pool).await;
    let editor = holder(&pool, "flow.fca.update").await;
    save_order(&state, &id, &editor, &ORDER).await;

    let (status, body) = send_json(&state, Method::GET, "/api/v1/public/flight/GND1", "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let fcas = body["fcas"].as_array().expect("fcas");
    assert_eq!(fcas.len(), 1, "{body}");
    assert_eq!(fcas[0]["fca_id"], id.as_str());
    assert_eq!(
        fcas[0]["seq"], 3,
        "merged behind NEAR1, not at the tail: {body}"
    );
}
