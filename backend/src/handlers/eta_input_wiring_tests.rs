//! #813 AC2: every FCA path, airport flow and the per-fix table time a ground flight with the
//! departure runway stored in `flow.departure_runway_assignment`, the map `list_idst` reads. Most
//! tests go through the real router before and after the assignment is stored, and the surface's
//! time moves by the change in the ground allowance the assignment makes (within 3 s, the time
//! between the two requests); the demand test asserts the count that change moves. The last test
//! checks that the badge count and the advisory read the board's path. The `TimingTables::load`
//! tests at the end do the same for the other tables it hands the predictor: the aircraft
//! profiles, the gate catalog and the replay's historical winds.
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

/// #813: the historical replay times a past instant, and a departure-runway assignment is live
/// state, so `GET /api/v1/stats/hist/flow/{icao}` times GND1 with the airport-wide allowance
/// (50 + 40 + 600 s, the median of every KJFK sample) even with 04L stored for its callsign,
/// while the live airport flow moves to 04L's (positive control).
#[sqlx::test]
async fn the_historical_flow_replay_ignores_a_stored_departure_runway(pool: PgPool) {
    let state = state(pool.clone()).await;
    let shift = shift_sec(&state).await;
    let at = DateTime::parse_from_rfc3339("2026-10-15T18:30:00Z")
        .unwrap()
        .with_timezone(&Utc);
    sqlx::query(
        "insert into stats.flight (session_id, cid, callsign, logon_time, first_seen, last_seen, \
             status, departure, arrival, aircraft_short, route, cruise_alt, cruise_tas) \
         values (1, 1000001, 'GND1', $1, $1, $2, 'active', 'KJFK', 'KDCA', 'B738', \
             'RBV WHITE SIE', 35000, 440)",
    )
    .bind(at - chrono::Duration::hours(1))
    .bind(at + chrono::Duration::hours(1))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "insert into stats.position (session_id, ts, lat, lon, altitude, groundspeed, heading) \
         values (1, $1, 40.64, -73.78, 13, 0, 310)",
    )
    .bind(at - chrono::Duration::minutes(1))
    .execute(&pool)
    .await
    .unwrap();
    let hist = format!("/api/v1/stats/hist/flow/KDCA?at={}", at.timestamp());
    let live = "/api/v1/tmu/flow/KDCA";
    let flow_reader = holder(&pool, "tmu.program.read").await;
    let (status, live_before) = send_json(&state, Method::GET, live, &flow_reader).await;
    assert_eq!(status, 200, "{live_before}");
    assign_04l(&pool).await;

    let member = session_cookie(&pool, &seed_user(&pool).await).await;
    let (status, body) = send_json(&state, Method::GET, &hist, &member).await;
    assert_eq!(status, 401, "no stats.data.read: {body}");

    let stats_reader = holder(&pool, "stats.data.read").await;
    let (status, body) = send_json(&state, Method::GET, &hist, &stats_reader).await;
    assert_eq!(status, 200, "{body}");
    let row = gnd1_in(&body["flights"]);
    assert_eq!(row["status"], "ground", "{row}");
    assert_eq!(
        time(&row["etd"]),
        at + chrono::Duration::seconds(50 + 40 + 600),
        "the airport-wide allowance, not 04L's 50 + 40 + 900 s"
    );

    let (_, live_after) = send_json(&state, Method::GET, live, &flow_reader).await;
    assert_shift(
        time(&gnd1_in(&live_before["flights"])["etd"]),
        time(&gnd1_in(&live_after["flights"])["etd"]),
        shift,
        "positive control: live airport-flow wheels-up",
    );
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

/// #813: a parked flight whose filed route does not resolve is timed on the great circle to its
/// destination, padded by 1.12 because it is not yet airborne. Its FCA row shows the distance that
/// ETA is timed over, and its route popup draws the same great circle the FCA map does, not a dot.
#[sqlx::test]
async fn a_parked_flight_on_an_unresolved_route_shows_its_timed_distance_and_path(pool: PgPool) {
    let state = state(pool.clone()).await;
    let parked = Pilot {
        callsign: "LOST2".into(),
        latitude: 40.2,
        longitude: -72.5,
        altitude: 0,
        groundspeed: 0,
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
    state.feed.write().await.snapshot = Some(Arc::new(crate::feed::Snapshot::of(VatsimData {
        pilots: vec![parked],
        ..Default::default()
    })));
    let id = corridor_fca(&pool).await;
    let (status, board) = send_json(
        &state,
        Method::GET,
        &format!("/api/v1/flow/fcas/{id}/traffic"),
        "",
    )
    .await;
    assert_eq!(status, 200, "{board}");
    let row = board
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["callsign"] == "LOST2")
        .unwrap_or_else(|| panic!("the board meters it: {board}"));
    assert_eq!(row["status"], "ground", "{row}");
    let raw = crate::feed::flow::gc_dist(
        40.2,
        -72.5,
        row["cross_lat"].as_f64().unwrap(),
        row["cross_lon"].as_f64().unwrap(),
    );
    assert!(
        raw > 20.0,
        "positive control: padding moves it by whole miles: {raw}"
    );
    assert_eq!(
        row["distance_nm"],
        (raw * predict::GROUND_ROUTE_FACTOR).round() as i64,
        "the padded distance the ETA is timed over, not the raw {raw}"
    );

    let (status, route) =
        send_json(&state, Method::GET, "/api/v1/flow/aircraft/LOST2/route", "").await;
    assert_eq!(status, 200, "{route}");
    assert_eq!(
        route["points"],
        json!([[40.2, -72.5], [38.85, -77.04]]),
        "the great circle to the destination: {route}"
    );
}

// ---- the other tables `TimingTables::load` hands the predictor --------------------------------

/// The tables [`gnd1_times`] takes in place of the state's: the aircraft profiles, the gate
/// catalog, the assigned runways and the winds.
struct Given<'a> {
    profiles: &'a crate::feed::trajectory::ProfileTable,
    gates: &'a HashMap<String, Vec<crate::models::AirportGateBody>>,
    assigned: &'a predict::AssignedRunways,
    winds: &'a crate::feed::winds::Winds,
}

/// GND1's arrival ETA and wheels-up from the predictor over the state's navigation, airports,
/// runways and taxi samples and the tables `given`, at `now`.
async fn gnd1_times(
    state: &AppState,
    given: Given<'_>,
    now: DateTime<Utc>,
) -> (DateTime<Utc>, DateTime<Utc>) {
    let Given {
        profiles,
        gates,
        assigned,
        winds,
    } = given;
    let (nav, airports) = (
        state.nav.load_full(),
        state.feed.read().await.airports.clone(),
    );
    let samples = state.taxi_estimate_samples.load_full();
    let inp = predict::Inputs {
        nav: &nav,
        airports: &airports,
        winds,
        profiles,
        ground: predict::GroundInputs {
            gates,
            runways: &state.runways,
            taxi_samples: &samples,
            assigned,
        },
    };
    let p = gnd1();
    let traj = predict::trajectory(&inp, predict::Flight::Pilot(&p), None, now).unwrap();
    (traj.eta_at_field(), traj.wheels_up())
}

/// A B738 profile that climbs, cruises and descends slower than the default, so GND1 lands later.
fn slow_b738() -> crate::feed::trajectory::ProfileTable {
    use crate::feed::trajectory::{AircraftProfile, ProfileTable};
    let b738 = AircraftProfile {
        climb_ias_hi: 250.0,
        climb_fpm_hi: 900.0,
        cruise_tas: Some(330.0),
        desc_ias_hi: 240.0,
        ..AircraftProfile::default()
    };
    ProfileTable::new(
        AircraftProfile::default(),
        HashMap::new(),
        HashMap::from([("B738".to_string(), b738)]),
    )
}

/// #813: a stored aircraft profile reaches the surfaces that time through `TimingTables::load`:
/// airport flow lands GND1 where the predictor puts it on its type's profile, and the FCA board
/// moves it later with it.
#[sqlx::test]
async fn airport_flow_and_the_fca_board_time_a_flight_on_its_type_profile(pool: PgPool) {
    let state = state(pool.clone()).await;
    let (default, typed) = (Default::default(), slow_b738());
    let (gates, assigned, winds) = (HashMap::new(), HashMap::new(), Default::default());
    let given = |profiles| Given {
        profiles,
        gates: &gates,
        assigned: &assigned,
        winds: &winds,
    };
    let now = Utc::now();
    let (eta_default, _) = gnd1_times(&state, given(&default), now).await;
    let (eta_typed, _) = gnd1_times(&state, given(&typed), now).await;
    let shift = (eta_typed - eta_default).num_seconds();
    assert!(
        shift >= 60,
        "positive control: the profile moves the ETA by {shift} s"
    );

    let id = corridor_fca(&pool).await;
    let cookie = holder(&pool, "tmu.program.read").await;
    let (flow, board) = (
        "/api/v1/tmu/flow/KDCA",
        format!("/api/v1/flow/fcas/{id}/traffic"),
    );
    let (status, flow_before) = send_json(&state, Method::GET, flow, &cookie).await;
    assert_eq!(status, 200, "{flow_before}");
    let (_, board_before) = send_json(&state, Method::GET, &board, "").await;
    state.aircraft_profiles.store(Arc::new(slow_b738()));
    let (_, flow_after) = send_json(&state, Method::GET, flow, &cookie).await;
    let (_, board_after) = send_json(&state, Method::GET, &board, "").await;

    assert_shift(
        time(&gnd1_in(&flow_before["flights"])["eta"]),
        time(&gnd1_in(&flow_after["flights"])["eta"]),
        shift,
        "airport-flow ETA on the B738 profile",
    );
    let crossing = |b: &Value| time(&gnd1_in(b)["eta"]);
    let moved = (crossing(&board_after) - crossing(&board_before)).num_seconds();
    assert!(
        moved >= 60,
        "the FCA crossing moves later with the profile: {moved} s"
    );
}

/// #813: the gate catalog reaches the ground allowance through `TimingTables::load`: GND1 parked
/// on stand G1 with 04L assigned, where five learned samples share its stand, type and runway,
/// takes that tier's allowance on airport flow, and the FCA board names the stand.
#[sqlx::test]
async fn airport_flow_and_the_fca_board_time_a_departure_from_its_stand(pool: PgPool) {
    let state = state(pool.clone()).await;
    let stand = crate::models::AirportGateBody {
        id: "G1".into(),
        icao: "KJFK".into(),
        name: "G1".into(),
        lat: 40.64,
        lon: -73.78,
        source: "manual".into(),
        kind: None,
        heading: None,
        size_code: None,
        operation_type: None,
        aircraft_classes: None,
        airline_codes: None,
        updated_at: Utc::now(),
        editable: false,
    };
    let mut samples = state.taxi_estimate_samples.load_full().as_ref().clone();
    samples
        .get_mut("KJFK")
        .unwrap()
        .extend((0..5).map(|_| TaxiSample {
            gate_id: Some("G1".into()),
            aircraft: Some("B738".into()),
            runway: Some("04L".into()),
            pushback_sec: Some(300),
            startup_sec: Some(200),
            taxi_sec: 1_200,
        }));
    state.taxi_estimate_samples.store(Arc::new(samples));
    let gates = HashMap::from([("KJFK".to_string(), vec![stand])]);
    let (profiles, no_gates, winds) = (Default::default(), HashMap::new(), Default::default());
    let assigned = predict::AssignedRunways::from([(
        ("KJFK".to_string(), "GND1".to_string()),
        ("04L".to_string(), "auto".to_string()),
    )]);
    let given = |gates| Given {
        profiles: &profiles,
        gates,
        assigned: &assigned,
        winds: &winds,
    };
    let now = Utc::now();
    let (_, up_runway) = gnd1_times(&state, given(&no_gates), now).await;
    let (_, up_stand) = gnd1_times(&state, given(&gates), now).await;
    let shift = (up_stand - up_runway).num_seconds();
    assert!(
        shift >= 60,
        "positive control: the stand moves wheels-up by {shift} s"
    );

    let id = corridor_fca(&pool).await;
    assign_04l(&pool).await;
    let cookie = holder(&pool, "tmu.program.read").await;
    let (flow, board) = (
        "/api/v1/tmu/flow/KDCA",
        format!("/api/v1/flow/fcas/{id}/traffic?debug=true"),
    );
    let (status, flow_before) = send_json(&state, Method::GET, flow, &cookie).await;
    assert_eq!(status, 200, "{flow_before}");
    let (_, board_before) = send_json(&state, Method::GET, &board, "").await;
    state.gates.store(Arc::new(gates));
    let (_, flow_after) = send_json(&state, Method::GET, flow, &cookie).await;
    let (_, board_after) = send_json(&state, Method::GET, &board, "").await;

    assert_shift(
        time(&gnd1_in(&flow_before["flights"])["etd"]),
        time(&gnd1_in(&flow_after["flights"])["etd"]),
        shift,
        "airport-flow wheels-up from stand G1",
    );
    assert_eq!(
        gnd1_in(&board_before)["debug"]["taxi_estimate"]["gate"],
        Value::Null
    );
    assert_eq!(
        gnd1_in(&board_after)["debug"]["taxi_estimate"]["gate"],
        "G1"
    );
    assert_shift(
        time(&gnd1_in(&board_before)["eta"]),
        time(&gnd1_in(&board_after)["eta"]),
        shift,
        "FCA crossing from stand G1",
    );
}

/// #813: the live winds reach the surfaces that time through `TimingTables::live` and the route
/// popup's per-fix table: airport flow and the last fix land GND1 where the predictor puts it in
/// those winds, not in still air.
#[sqlx::test]
async fn airport_flow_and_the_fix_table_time_a_flight_in_the_live_winds(pool: PgPool) {
    let state = state(pool.clone()).await;
    let (shear, still) = (crate::feed::winds::Winds::test_shear(), Default::default());
    let (profiles, gates, assigned) = (Default::default(), HashMap::new(), HashMap::new());
    let given = |winds| Given {
        profiles: &profiles,
        gates: &gates,
        assigned: &assigned,
        winds,
    };
    let now = Utc::now();
    let (eta_still, _) = gnd1_times(&state, given(&still), now).await;
    let (eta_shear, _) = gnd1_times(&state, given(&shear), now).await;
    assert!(
        (eta_shear - eta_still).num_seconds().abs() >= 30,
        "positive control: the winds move the ETA: {eta_shear} vs {eta_still}"
    );
    state
        .winds
        .store(Arc::new(crate::feed::winds::Winds::test_shear()));

    let cookie = holder(&pool, "tmu.program.read").await;
    let (status, flow) = send_json(&state, Method::GET, "/api/v1/tmu/flow/KDCA", &cookie).await;
    assert_eq!(status, 200, "{flow}");
    let (status, route) =
        send_json(&state, Method::GET, "/api/v1/flow/aircraft/GND1/route", "").await;
    assert_eq!(status, 200, "{route}");
    let last = &route["fixes"].as_array().unwrap().last().unwrap();
    assert_eq!(last["name"], "KDCA", "{route}");
    for (surface, eta) in [
        ("airport flow", time(&gnd1_in(&flow["flights"])["eta"])),
        ("per-fix table", time(&last["eta"])),
    ] {
        assert!(
            (eta - eta_shear).num_seconds().abs() <= 3,
            "{surface} ETA {eta}: the live winds give {eta_shear}, still air {eta_still}"
        );
    }
}

/// #813: the flow and departures replays time the past in the winds stored for that instant,
/// which `TimingTables::load` takes from its caller, not in the live winds.
#[sqlx::test]
async fn the_historical_replays_time_a_flight_in_the_winds_of_its_instant(pool: PgPool) {
    let state = state(pool.clone()).await;
    let at = DateTime::parse_from_rfc3339("2026-10-16T01:30:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let (shear, still) = (crate::feed::winds::Winds::test_shear(), Default::default());
    let (profiles, gates, assigned) = (Default::default(), HashMap::new(), HashMap::new());
    let given = |winds| Given {
        profiles: &profiles,
        gates: &gates,
        assigned: &assigned,
        winds,
    };
    let (eta_still, _) = gnd1_times(&state, given(&still), at).await;
    let (eta_shear, _) = gnd1_times(&state, given(&shear), at).await;
    assert!(
        (eta_shear - eta_still).num_seconds().abs() >= 30,
        "positive control: the stored winds move the ETA: {eta_shear} vs {eta_still}"
    );
    crate::repos::stats::upsert_winds(&pool, at - chrono::Duration::minutes(10), &shear)
        .await
        .unwrap();
    sqlx::query(
        "insert into stats.flight (session_id, cid, callsign, logon_time, first_seen, last_seen, \
             status, departure, arrival, aircraft_short, route, cruise_alt, cruise_tas) \
         values (1, 1000001, 'GND1', $1, $1, $2, 'active', 'KJFK', 'KDCA', 'B738', \
             'RBV WHITE SIE', 35000, 440)",
    )
    .bind(at - chrono::Duration::hours(1))
    .bind(at + chrono::Duration::hours(1))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "insert into stats.position (session_id, ts, lat, lon, altitude, groundspeed, heading) \
         values (1, $1, 40.64, -73.78, 13, 0, 310)",
    )
    .bind(at - chrono::Duration::minutes(1))
    .execute(&pool)
    .await
    .unwrap();

    let cookie = holder(&pool, "stats.data.read").await;
    let uri = format!("/api/v1/stats/hist/flow/KDCA?at={}", at.timestamp());
    let (status, body) = send_json(&state, Method::GET, &uri, &cookie).await;
    assert_eq!(status, 200, "{body}");
    let eta = time(&gnd1_in(&body["flights"])["eta"]);
    assert!(
        (eta - eta_shear).num_seconds().abs() <= 1,
        "flow replay ETA {eta}: the stored winds give {eta_shear}, still air {eta_still}"
    );

    // The departures replay times KJFK's departures to a metered field the same way.
    sqlx::query("insert into tmu.programs (icao, aar) values ('KDCA', 60)")
        .execute(&pool)
        .await
        .unwrap();
    let uri = format!("/api/v1/stats/hist/departures/KJFK?at={}", at.timestamp());
    let (status, body) = send_json(&state, Method::GET, &uri, &cookie).await;
    assert_eq!(status, 200, "{body}");
    let row = gnd1_in(&body["departures"]);
    assert_eq!(
        row["has_program"], true,
        "positive control: KDCA is metered"
    );
    let eta = time(&row["eta"]);
    assert!(
        (eta - eta_shear).num_seconds().abs() <= 1,
        "departures replay ETA {eta}: the stored winds give {eta_shear}, still air {eta_still}"
    );
}
