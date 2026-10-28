//! #813 AC2: every FCA path, airport flow and the per-fix table time a ground flight with the
//! departure runway stored in `flow.departure_runway_assignment`, the map `list_idst` reads. Most
//! tests go through the real router before and after the assignment is stored, and the surface's
//! time moves by the change in the ground allowance the assignment makes (within 3 s, the time
//! between the two requests); the demand test asserts the count that change moves. The last test
//! checks that the badge count and the advisory read the board's path.
//!
//! The feed: GND1 parked at KJFK, filed KJFK->KDCA via `RBV WHITE SIE`, crossing an FCA on the
//! corridor between WHITE and SIE. KJFK has five taxi samples off 04L (900 s) and five off 31L
//! (300 s), so the assignment of 04L reaches the runway tier.

use std::collections::HashMap;
use std::sync::Arc;

use axum::http::Method;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::feed::airports::Airport;
use crate::feed::predict;
use crate::feed::taxi_estimate::TaxiSample;
use crate::feed::vatsim::{FlightPlan, Pilot, VatsimData};
use crate::handlers::feed::TimingTables;
use crate::repos::departure_runway::{self as departure_runway_repo, RunwaySource};
use crate::scope_test_support::{artcc, grant, seed_user, send_json, session_cookie, test_state};
use crate::state::AppState;

fn gnd1() -> Pilot {
    Pilot {
        callsign: "GND1".into(),
        latitude: 40.64,
        longitude: -73.78,
        altitude: 13,
        groundspeed: 0,
        heading: 310,
        flight_plan: Some(FlightPlan {
            departure: "KJFK".into(),
            arrival: "KDCA".into(),
            route: "RBV WHITE SIE".into(),
            aircraft_short: "B738".into(),
            altitude: "35000".into(),
            cruise_tas: "440".into(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

async fn state(pool: PgPool) -> AppState {
    let state = test_state(pool, HashMap::from([("ZNY".to_string(), artcc(&["KJFK"]))]));
    {
        let mut feed = state.feed.write().await;
        feed.airports = Arc::new(HashMap::from([
            ("KJFK".to_string(), Airport::at(40.64, -73.78)),
            ("KDCA".to_string(), Airport::at(38.85, -77.04)),
        ]));
        feed.snapshot = Some(Arc::new(crate::feed::Snapshot::of(VatsimData {
            pilots: vec![gnd1()],
            ..Default::default()
        })));
    }
    let sample = |runway: &str, taxi_sec| TaxiSample {
        gate_id: None,
        aircraft: None,
        runway: Some(runway.into()),
        pushback_sec: Some(50),
        startup_sec: Some(40),
        taxi_sec,
    };
    let mut at_jfk: Vec<_> = (0..5).map(|_| sample("04L", 900)).collect();
    at_jfk.extend((0..5).map(|_| sample("31L", 300)));
    state
        .taxi_estimate_samples
        .store(Arc::new(HashMap::from([("KJFK".to_string(), at_jfk)])));
    state
}

/// How much later the assignment makes GND1 leave: its allowance with 04L less its allowance
/// with no runway, from the predictor over the state's own tables.
async fn shift_sec(state: &AppState) -> i64 {
    let assigned = predict::AssignedRunways::from([(
        ("KJFK".to_string(), "GND1".to_string()),
        ("04L".to_string(), "auto".to_string()),
    )]);
    let allowance = |tables: &TimingTables| {
        let p = gnd1();
        predict::trajectory(
            &tables.inputs(),
            predict::Flight::Pilot(&p),
            None,
            Utc::now(),
        )
        .unwrap()
        .allowance_sec()
    };
    let with = TimingTables::load(state, state.winds.load_full(), Arc::new(assigned)).await;
    let without = TimingTables::load(state, state.winds.load_full(), Arc::default()).await;
    let (a, b) = (allowance(&with), allowance(&without));
    assert_eq!(
        a,
        50.0 + 40.0 + 900.0,
        "positive control: 04L's runway tier"
    );
    assert!(
        a - b >= 60.0,
        "the assignment moves the allowance: {a} vs {b}"
    );
    (a - b) as i64
}

async fn assign_04l(pool: &PgPool) {
    assert!(
        departure_runway_repo::assign(pool, "KJFK", "GND1", "04L", RunwaySource::Auto, None)
            .await
            .unwrap()
    );
}

/// An enabled rate FCA on the corridor, and its id.
async fn corridor_fca(pool: &PgPool) -> String {
    sqlx::query_scalar(
        "insert into flow.fca (name, color, artcc, points, dests, origins, fixes, scope, \
             dir, mode, rate, mit, enabled) \
         values ('C', '#fff', 'ZDC', $1, '{}', '{}', '{}', '{}', 'any', 'rate', 30, 0, true) \
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

fn time(v: &Value) -> DateTime<Utc> {
    v.as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .unwrap_or_else(|| panic!("not a time: {v}"))
        .with_timezone(&Utc)
}

fn gnd1_in(list: &Value) -> &Value {
    list.as_array()
        .unwrap()
        .iter()
        .find(|f| f["callsign"] == "GND1")
        .unwrap_or_else(|| panic!("GND1 missing: {list}"))
}

/// The later reading less the earlier one, in seconds, against the expected shift: the two
/// requests run a moment apart, so their `now` differs by that moment.
fn assert_shift(before: DateTime<Utc>, after: DateTime<Utc>, shift: i64, surface: &str) {
    let moved = (after - before).num_seconds();
    assert!(
        (moved - shift).abs() <= 3,
        "{surface}: moved {moved} s, the assignment's shift is {shift} s"
    );
}

#[sqlx::test]
async fn fca_traffic_times_a_departure_from_its_assigned_runway(pool: PgPool) {
    let state = state(pool.clone()).await;
    let shift = shift_sec(&state).await;
    let id = corridor_fca(&pool).await;
    let uri = format!("/api/v1/flow/fcas/{id}/traffic?debug=true");
    let (status, before) = send_json(&state, Method::GET, &uri, "").await;
    assert_eq!(status, 200, "{before}");
    assign_04l(&pool).await;
    let (_, after) = send_json(&state, Method::GET, &uri, "").await;
    let (b, a) = (gnd1_in(&before), gnd1_in(&after));
    assert_eq!(a["debug"]["taxi_estimate"]["runway"], "04L");
    assert_eq!(a["debug"]["taxi_estimate"]["taxi_sec"], 900);
    assert_eq!(b["debug"]["taxi_estimate"]["runway"], Value::Null);
    assert_shift(time(&b["eta"]), time(&a["eta"]), shift, "FCA traffic");
}

#[sqlx::test]
async fn idst_proposes_a_release_from_the_assigned_runway(pool: PgPool) {
    let state = state(pool.clone()).await;
    let shift = shift_sec(&state).await;
    corridor_fca(&pool).await;
    let cookie = holder(&pool, "flow.fca.read").await;
    let uri = "/api/v1/flow/idst?airports=KJFK";
    let (status, before) = send_json(&state, Method::GET, uri, &cookie).await;
    assert_eq!(status, 200, "{before}");
    assign_04l(&pool).await;
    let (_, after) = send_json(&state, Method::GET, uri, &cookie).await;
    let (b, a) = (
        gnd1_in(&before["unscheduled"]),
        gnd1_in(&after["unscheduled"]),
    );
    assert_eq!(a["runway"], "04L");
    assert_eq!(
        b["runway"],
        Value::Null,
        "positive control: no configuration predicts one"
    );
    assert_shift(
        time(&b["cross_time"]),
        time(&a["cross_time"]),
        shift,
        "IDST crossing",
    );
}

/// One request with a JSON body through the real router; `send_json` sends none.
async fn call(
    state: &AppState,
    method: Method,
    uri: &str,
    cookie: &str,
    body: Option<Value>,
) -> Value {
    use tower::ServiceExt;

    let builder = axum::http::Request::builder()
        .method(method)
        .uri(uri)
        .header(axum::http::header::COOKIE, cookie);
    let request = match body {
        Some(b) => builder
            .header(axum::http::header::CONTENT_TYPE, "application/json")
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
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    assert_eq!(status, 200, "{uri}: {body}");
    body
}

#[sqlx::test]
async fn a_rdy_release_and_a_clear_time_the_departure_from_its_assigned_runway(pool: PgPool) {
    let state = state(pool.clone()).await;
    let shift = shift_sec(&state).await;
    let id = corridor_fca(&pool).await;
    let cookie = holder(&pool, "flow.fca.update").await;
    let uri = format!("/api/v1/flow/fcas/{id}/release/GND1");
    // RDY with no ready time, then clear it: the wheels-up frozen, then the unreleased ETA.
    let rdy_then_clear = || async {
        let released = call(&state, Method::POST, &uri, &cookie, Some(json!({}))).await;
        let cleared = call(&state, Method::DELETE, &uri, &cookie, None).await;
        (
            time(&gnd1_in(&released)["edct"]),
            time(&gnd1_in(&cleared)["eta"]),
        )
    };
    let (edct_b, eta_b) = rdy_then_clear().await;
    assign_04l(&pool).await;
    let (edct_a, eta_a) = rdy_then_clear().await;
    assert_shift(edct_b, edct_a, shift, "RDY wheels-up");
    assert_shift(eta_b, eta_a, shift, "clear");
}

#[sqlx::test]
async fn airport_flow_times_a_departure_from_its_assigned_runway(pool: PgPool) {
    let state = state(pool.clone()).await;
    let shift = shift_sec(&state).await;
    let cookie = holder(&pool, "tmu.program.read").await;
    let uri = "/api/v1/tmu/flow/KDCA";
    let (status, before) = send_json(&state, Method::GET, uri, &cookie).await;
    assert_eq!(status, 200, "{before}");
    assign_04l(&pool).await;
    let (_, after) = send_json(&state, Method::GET, uri, &cookie).await;
    let (b, a) = (gnd1_in(&before["flights"]), gnd1_in(&after["flights"]));
    assert_shift(
        time(&b["etd"]),
        time(&a["etd"]),
        shift,
        "airport-flow wheels-up",
    );
    assert_shift(time(&b["eta"]), time(&a["eta"]), shift, "airport-flow ETA");
}

#[sqlx::test]
async fn the_fix_table_times_a_departure_from_its_assigned_runway(pool: PgPool) {
    let state = state(pool.clone()).await;
    let shift = shift_sec(&state).await;
    let uri = "/api/v1/flow/aircraft/GND1/route";
    let (status, before) = send_json(&state, Method::GET, uri, "").await;
    assert_eq!(status, 200, "{before}");
    assign_04l(&pool).await;
    let (_, after) = send_json(&state, Method::GET, uri, "").await;
    let last = |v: &Value| time(&v["fixes"].as_array().unwrap().last().unwrap()["eta"]);
    assert_eq!(
        after["fixes"].as_array().unwrap().last().unwrap()["name"],
        "KDCA"
    );
    assert_shift(last(&before), last(&after), shift, "per-fix table");
}

#[sqlx::test]
async fn the_departures_list_times_a_departure_from_its_assigned_runway(pool: PgPool) {
    let state = state(pool.clone()).await;
    let shift = shift_sec(&state).await;
    sqlx::query("insert into tmu.programs (icao, aar) values ('KDCA', 60)")
        .execute(&pool)
        .await
        .unwrap();
    let cookie = holder(&pool, "tmu.program.read").await;
    let uri = "/api/v1/tmu/departures/KJFK";
    let (status, before) = send_json(&state, Method::GET, uri, &cookie).await;
    assert_eq!(status, 200, "{before}");
    assign_04l(&pool).await;
    let (_, after) = send_json(&state, Method::GET, uri, &cookie).await;
    let (b, a) = (
        gnd1_in(&before["departures"]),
        gnd1_in(&after["departures"]),
    );
    assert_eq!(a["has_program"], true, "positive control: KDCA is metered");
    assert_shift(time(&b["eta"]), time(&a["eta"]), shift, "departures ETA");
}

/// A CFR issued with a ready time is slotted behind every other metered arrival. At AAR 1, GND1
/// holds KDCA's first slot and PRE2 (prefiled, ready now) goes an hour behind it, so PRE2's locked
/// wheels-up follows GND1's arrival, which the assignment moves.
#[sqlx::test]
async fn a_cfr_with_a_ready_time_slots_behind_an_assigned_departure(pool: PgPool) {
    let state = state(pool.clone()).await;
    let shift = shift_sec(&state).await;
    {
        let mut feed = state.feed.write().await;
        let data = VatsimData {
            pilots: vec![gnd1()],
            prefiles: vec![crate::feed::vatsim::Prefile {
                callsign: "PRE2".into(),
                flight_plan: gnd1().flight_plan,
                ..Default::default()
            }],
            ..Default::default()
        };
        feed.snapshot = Some(Arc::new(crate::feed::Snapshot::of(data)));
    }
    sqlx::query("insert into tmu.programs (icao, aar) values ('KDCA', 1)")
        .execute(&pool)
        .await
        .unwrap();
    let cookie = holder(&pool, "tmu.cfr.assign").await;
    let issue = || async {
        let body = json!({"callsign": "PRE2", "airport": "KDCA", "ready_time": Utc::now()});
        let cfr = call(&state, Method::POST, "/api/v1/tmu/cfr", &cookie, Some(body)).await;
        time(&cfr["wheels_up"])
    };
    let before = issue().await;
    assert!(
        before > Utc::now() + chrono::Duration::minutes(50),
        "positive control: slotted an hour behind GND1, not at its ready time: {before}"
    );
    // Withdraw PRE2's CFR, so the second issue is slotted against the same picture as the first.
    sqlx::query("delete from tmu.issued_cfrs where callsign = 'PRE2'")
        .execute(&pool)
        .await
        .unwrap();
    assign_04l(&pool).await;
    let after = issue().await;
    assert_shift(before, after, shift, "CFR ready-time slot");
}

/// Demand counts arrivals due inside 60 minutes. The arrival field is placed so GND1 lands inside
/// the hour from the airport-tier allowance and outside it from its assigned runway.
#[sqlx::test]
async fn demand_counts_a_departure_from_its_assigned_runway(pool: PgPool) {
    let state = state(pool.clone()).await;
    let shift = shift_sec(&state).await;
    let assigned = predict::AssignedRunways::from([(
        ("KJFK".to_string(), "GND1".to_string()),
        ("04L".to_string(), "auto".to_string()),
    )]);
    let mut field = None;
    for step in 0..400 {
        let lon = -74.5 - f64::from(step) * 0.05;
        state.feed.write().await.airports = Arc::new(HashMap::from([
            ("KJFK".to_string(), Airport::at(40.64, -73.78)),
            ("KXYZ".to_string(), Airport::at(40.64, lon)),
        ]));
        let p = Pilot {
            flight_plan: Some(FlightPlan {
                arrival: "KXYZ".into(),
                ..gnd1().flight_plan.unwrap()
            }),
            ..gnd1()
        };
        let mins = |tables: TimingTables| {
            let now = Utc::now();
            let inp = tables.inputs();
            let t = predict::trajectory(&inp, predict::Flight::Pilot(&p), None, now).unwrap();
            (t.eta_at_field() - now).num_seconds()
        };
        let without =
            mins(TimingTables::load(&state, state.winds.load_full(), Arc::default()).await);
        let with = mins(
            TimingTables::load(&state, state.winds.load_full(), Arc::new(assigned.clone())).await,
        );
        assert!((with - without - shift).abs() <= 1);
        if without <= 3_600 - shift / 2 + 30 && with >= 3_600 + 30 && without <= 3_600 - 30 {
            field = Some(p);
            break;
        }
    }
    let p = field.expect("a field straddling the hour");
    {
        let mut feed = state.feed.write().await;
        feed.snapshot = Some(Arc::new(crate::feed::Snapshot::of(VatsimData {
            pilots: vec![p],
            ..Default::default()
        })));
    }
    sqlx::query("insert into tmu.programs (icao, aar) values ('KXYZ', 30)")
        .execute(&pool)
        .await
        .unwrap();
    let cookie = holder(&pool, "tmu.program.read").await;
    let demand = |rows: &Value| {
        rows.as_array()
            .unwrap()
            .iter()
            .find(|r| r["icao"] == "KXYZ")
            .unwrap_or_else(|| panic!("KXYZ missing: {rows}"))["demand_60min"]
            .clone()
    };
    let (status, before) = send_json(&state, Method::GET, "/api/v1/tmu/demand", &cookie).await;
    assert_eq!(status, 200, "{before}");
    assert_eq!(
        demand(&before),
        1,
        "inside the hour on the airport-tier allowance"
    );
    assign_04l(&pool).await;
    let (_, after) = send_json(&state, Method::GET, "/api/v1/tmu/demand", &cookie).await;
    assert_eq!(demand(&after), 0, "outside it from the assigned runway");
}

/// A flight whose filed route does not resolve (an unknown departure, filed `DCT`) is timed on the
/// great circle to its destination. The metering board, the public badge count and the pilot's
/// own advisory all read that one path, so all three agree that it crosses the FCA.
#[sqlx::test]
async fn the_board_the_badge_count_and_the_advisory_agree_on_an_unresolved_route(pool: PgPool) {
    let state = state(pool.clone()).await;
    let lost = Pilot {
        callsign: "LOST1".into(),
        latitude: 40.2,
        longitude: -72.5,
        altitude: 30_000,
        groundspeed: 400,
        heading: 250,
        flight_plan: Some(FlightPlan {
            departure: "KZZZ".into(),
            arrival: "KDCA".into(),
            route: "DCT".into(),
            aircraft_short: "B738".into(),
            altitude: "30000".into(),
            cruise_tas: "440".into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    {
        let mut feed = state.feed.write().await;
        assert!(
            crate::feed::fca::route_path(
                state.nav.load().as_ref(),
                &feed.airports,
                "KZZZ",
                "KDCA",
                "DCT",
                40.2,
                -72.5,
                250,
                400
            )
            .is_none(),
            "positive control: the filed route does not resolve"
        );
        feed.snapshot = Some(Arc::new(crate::feed::Snapshot::of(VatsimData {
            pilots: vec![lost],
            ..Default::default()
        })));
    }
    let id = corridor_fca(&pool).await;
    let (status, board) = send_json(
        &state,
        Method::GET,
        &format!("/api/v1/flow/fcas/{id}/traffic"),
        "",
    )
    .await;
    assert_eq!(status, 200, "{board}");
    let listed = board
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["callsign"] == "LOST1");
    assert!(listed, "the board meters it: {board}");
    let (_, counts) = send_json(&state, Method::GET, "/api/v1/flow/counts", "").await;
    assert_eq!(counts[&id], 1, "the badge counts it: {counts}");
    let (_, advisory) = send_json(&state, Method::GET, "/api/v1/public/flight/LOST1", "").await;
    let fcas: Vec<&Value> = advisory["fcas"].as_array().unwrap().iter().collect();
    assert!(
        fcas.iter().any(|f| f["fca_id"] == id.as_str()),
        "the advisory lists the FCA: {advisory}"
    );
}
