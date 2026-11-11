//! #813 AC2: the FCA board, the badge count and the advisory read one path for a flight whose route
//! does not resolve, and the tables `TimingTables` hands the predictor (the aircraft profiles, the
//! live winds and the replay's historical winds) reach the surfaces that time through it.
//!
//! The feed: GND1 parked at KJFK, filed KJFK->KDCA via `RBV WHITE SIE`, crossing an FCA on the
//! corridor between WHITE and SIE.

use std::collections::HashMap;
use std::sync::Arc;

use axum::http::Method;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::feed::airports::Airport;
use crate::feed::predict;
use crate::feed::vatsim::{FlightPlan, Pilot, VatsimData};
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
    state
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
        "{surface}: moved {moved} s, the expected shift is {shift} s"
    );
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

// ---- the tables `TimingTables` hands the predictor ---------------------------------------------

/// The tables [`gnd1_times`] takes in place of the state's: the aircraft profiles and the winds.
struct Given<'a> {
    profiles: &'a crate::feed::trajectory::ProfileTable,
    winds: &'a crate::feed::winds::Winds,
}

/// GND1's arrival ETA and wheels-up from the predictor over the state's navigation and airports and
/// the tables `given`, at `now`.
async fn gnd1_times(
    state: &AppState,
    given: Given<'_>,
    now: DateTime<Utc>,
) -> (DateTime<Utc>, DateTime<Utc>) {
    let Given { profiles, winds } = given;
    let (nav, airports) = (
        state.nav.load_full(),
        state.feed.read().await.airports.clone(),
    );
    let inp = predict::Inputs {
        nav: &nav,
        airports: &airports,
        winds,
        profiles,
        locked: &predict::NO_LOCKS,
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

/// #813: a stored aircraft profile reaches the surfaces that time through `TimingTables`:
/// airport flow lands GND1 where the predictor puts it on its type's profile, and the FCA board
/// moves it later with it.
#[sqlx::test]
async fn airport_flow_and_the_fca_board_time_a_flight_on_its_type_profile(pool: PgPool) {
    let state = state(pool.clone()).await;
    let (default, typed) = (Default::default(), slow_b738());
    let winds = Default::default();
    let given = |profiles| Given {
        profiles,
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

/// #813: the live winds reach the surfaces that time through `TimingTables::live` and the route
/// popup's per-fix table: airport flow and the last fix land GND1 where the predictor puts it in
/// those winds, not in still air.
#[sqlx::test]
async fn airport_flow_and_the_fix_table_time_a_flight_in_the_live_winds(pool: PgPool) {
    let state = state(pool.clone()).await;
    let (shear, still) = (crate::feed::winds::Winds::test_shear(), Default::default());
    let profiles = Default::default();
    let given = |winds| Given {
        profiles: &profiles,
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
    let profiles = Default::default();
    let given = |winds| Given {
        profiles: &profiles,
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
