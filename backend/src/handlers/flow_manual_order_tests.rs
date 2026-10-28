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

/// AC1 at the IDST caller: a ground departure at KJFK that the order does not name is sequenced by
/// ETA between an ordered arrival nearer the line and an ordered one still out by Boston.
#[sqlx::test]
async fn the_idst_list_sequences_a_late_joiner_at_its_merged_slot(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
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
    install_feed(&state, vec![near, ground, far]).await;
    let id = corridor_fca(&pool).await;
    let editor = holder(&pool, "flow.fca.update").await;
    let reader = holder(&pool, "flow.fca.read").await;

    // Auto first: the fixture's own ETA order puts GND1 between the other two.
    let rows = traffic(&state, &id).await;
    assert_eq!(
        field(&rows, "callsign"),
        [&json!("NEAR1"), &json!("GND1"), &json!("FAR1")],
        "fixture: {rows:?}"
    );
    assert_eq!(
        send(
            &state,
            Method::PUT,
            &format!("/api/v1/flow/fcas/{id}/order"),
            &editor,
            Some(json!({ "order": ["NEAR1", "FAR1"] })),
        )
        .await,
        StatusCode::NO_CONTENT
    );

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
        row["seq"], 2,
        "merged behind NEAR1, not at the tail: {idst}"
    );
}
