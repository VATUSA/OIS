//! #814 AC2, AC4 and AC6 through the real router: a ground flight timed from its proposed wheels-up is
//! marked `wheels_up_proposed` on the FCA traffic and airport-flow routes, and a scheduler time it
//! holds (a release, a CFR with or without a ready time) clears the mark, is what it is timed from,
//! and is what gets stored. The departures list is in `handlers::feed::departures_release_tests`.

use std::collections::HashMap;
use std::sync::Arc;

use axum::http::{Method, StatusCode};
use chrono::{DateTime, Duration, Timelike, Utc};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::feed::airports::Airport;
use crate::feed::vatsim::{FlightPlan, Pilot, Prefile, VatsimData};
use crate::scope_test_support::{grant, seed_user, send, send_json, session_cookie, test_state};
use crate::state::AppState;

fn plan(deptime: &str) -> FlightPlan {
    FlightPlan {
        departure: "KJFK".into(),
        arrival: "KDCA".into(),
        route: "RBV WHITE SIE".into(),
        deptime: deptime.into(),
        ..Default::default()
    }
}

fn parked(callsign: &str, deptime: &str) -> Pilot {
    Pilot {
        callsign: callsign.into(),
        latitude: 40.64,
        longitude: -73.78,
        altitude: 13,
        heading: 310,
        flight_plan: Some(plan(deptime)),
        ..Default::default()
    }
}

/// Airborne south of KJFK on the same route, short of the corridor FCA.
fn airborne(callsign: &str) -> Pilot {
    Pilot {
        callsign: callsign.into(),
        latitude: 40.2,
        longitude: -74.0,
        altitude: 24_000,
        groundspeed: 400,
        heading: 220,
        flight_plan: Some(plan("")),
        ..Default::default()
    }
}

async fn state_with(pool: &PgPool, data: VatsimData) -> AppState {
    let state = test_state(pool.clone(), HashMap::new());
    let mut feed = state.feed.write().await;
    feed.airports = Arc::new(HashMap::from([
        ("KJFK".to_string(), Airport::at(40.64, -73.78)),
        ("KDCA".to_string(), Airport::at(38.85, -77.04)),
    ]));
    feed.snapshot = Some(Arc::new(crate::feed::Snapshot::of(data)));
    drop(feed);
    state
}

/// An enabled rate FCA on the JFK->DCA corridor between WHITE and SIE, and its id.
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

async fn holder(pool: &PgPool, permissions: &[&str]) -> String {
    let user = seed_user(pool).await;
    for p in permissions {
        grant(pool, &user, p, None).await;
    }
    session_cookie(pool, &user).await
}

/// One request with a cookie and optional headers and JSON body through the real router.
async fn call(
    state: &AppState,
    method: Method,
    uri: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> (StatusCode, Value) {
    use tower::ServiceExt;
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let request = match body {
        Some(b) => builder
            .header("content-type", "application/json")
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

fn row<'a>(list: &'a Value, callsign: &str) -> &'a Value {
    list.as_array()
        .unwrap()
        .iter()
        .find(|f| f["callsign"] == callsign)
        .unwrap_or_else(|| panic!("{callsign} missing: {list}"))
}

fn at(v: &Value) -> DateTime<Utc> {
    serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("{v} is not a time: {e}"))
}

#[sqlx::test]
async fn fca_traffic_marks_only_an_unreleased_ground_flight_proposed(pool: PgPool) {
    let data = VatsimData {
        pilots: vec![parked("GRD", ""), parked("REL", ""), airborne("AIR")],
        ..Default::default()
    };
    let state = state_with(&pool, data).await;
    let id = corridor_fca(&pool).await;
    let edct = (Utc::now() + Duration::hours(1)).timestamp_millis();
    sqlx::query(
        "insert into flow.fca_release (fca_id, callsign, cta_ms, edct_ms) values ($1, 'REL', $2, $3)",
    )
    .bind(&id)
    .bind(edct + 30 * 60_000)
    .bind(edct)
    .execute(&pool)
    .await
    .unwrap();

    let (status, board) = send_json(
        &state,
        Method::GET,
        &format!("/api/v1/flow/fcas/{id}/traffic"),
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{board}");
    for (cs, status, released, proposed) in [
        ("GRD", "ground", false, true),
        ("REL", "ground", true, false),
        ("AIR", "airborne", false, false),
    ] {
        let f = row(&board, cs);
        assert_eq!(f["status"], status, "{cs}: {f}");
        assert_eq!(f["released"], released, "{cs}: {f}");
        assert_eq!(f["wheels_up_proposed"], proposed, "{cs}: {f}");
    }
    // The released flight is timed from its release, not its proposed wheels-up (#814).
    let rel = row(&board, "REL");
    assert_eq!(at(&rel["edct"]).timestamp_millis(), edct, "{rel}");
    assert!(
        at(&rel["eta"]).timestamp_millis() > edct,
        "its ETA runs from the release an hour out: {rel}"
    );
    assert!(
        at(&row(&board, "GRD")["eta"]) < at(&rel["eta"]),
        "the unreleased one crosses first"
    );
}

#[sqlx::test]
async fn an_rdy_release_clears_the_proposed_mark_in_the_response_and_reread(pool: PgPool) {
    let data = VatsimData {
        pilots: vec![parked("GRD", "")],
        ..Default::default()
    };
    let state = state_with(&pool, data).await;
    let id = corridor_fca(&pool).await;
    let traffic = format!("/api/v1/flow/fcas/{id}/traffic");
    let (_, before) = send_json(&state, Method::GET, &traffic, "").await;
    assert_eq!(row(&before, "GRD")["wheels_up_proposed"], true, "{before}");

    let user = seed_user(&pool).await;
    grant(&pool, &user, "flow.fca.update", Some("ZDC")).await;
    let cookie = session_cookie(&pool, &user).await;
    let (status, released) = call(
        &state,
        Method::POST,
        &format!("/api/v1/flow/fcas/{id}/release/GRD"),
        &[("cookie", &cookie), ("if-none-match", "*")],
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{released}");
    let grd = row(&released, "GRD");
    assert_eq!(grd["released"], true, "{grd}");
    assert_eq!(grd["wheels_up_proposed"], false, "{grd}");
    assert_eq!(
        grd["eta"], grd["cross_time"],
        "timed from its release, it reaches the line at its CTA: {grd}"
    );

    let (_, after) = send_json(&state, Method::GET, &traffic, "").await;
    let grd = row(&after, "GRD");
    assert_eq!(grd["wheels_up_proposed"], false, "{grd}");
    let lag = (at(&grd["eta"]) - at(&grd["cross_time"]))
        .num_seconds()
        .abs();
    assert!(lag <= 1, "re-read from the stored release: {grd}");
}

#[sqlx::test]
async fn airport_flow_refuses_a_caller_without_program_read(pool: PgPool) {
    let state = state_with(&pool, VatsimData::default()).await;
    let uri = "/api/v1/tmu/flow/KDCA";
    assert_eq!(
        send(&state, Method::GET, uri, "", None).await,
        StatusCode::UNAUTHORIZED
    );
    let fca_reader = holder(&pool, &["flow.fca.read"]).await;
    assert_eq!(
        send(&state, Method::GET, uri, &fca_reader, None).await,
        StatusCode::UNAUTHORIZED
    );
    let reader = holder(&pool, &["tmu.program.read"]).await;
    let (status, body) = send_json(&state, Method::GET, uri, &reader).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["icao"], "KDCA");
}

#[sqlx::test]
async fn airport_flow_marks_only_unscheduled_ground_inbounds(pool: PgPool) {
    let data = VatsimData {
        pilots: vec![parked("GRD", ""), parked("ISS", ""), airborne("AIR")],
        ..Default::default()
    };
    let state = state_with(&pool, data).await;
    sqlx::query("insert into tmu.programs (icao, aar) values ('KDCA', 30)")
        .execute(&pool)
        .await
        .unwrap();
    let locked = Utc::now() + Duration::minutes(45);
    sqlx::query(
        "insert into tmu.issued_cfrs (callsign, airport, wheels_up) values ('ISS', 'KDCA', $1)",
    )
    .bind(locked)
    .execute(&pool)
    .await
    .unwrap();
    let reader = holder(&pool, &["tmu.program.read"]).await;
    let (status, flow) = send_json(&state, Method::GET, "/api/v1/tmu/flow/KDCA", &reader).await;
    assert_eq!(status, StatusCode::OK, "{flow}");
    let flights = &flow["flights"];
    let grd = row(flights, "GRD");
    assert_eq!(grd["wheels_up_proposed"], true, "{grd}");
    assert!(!grd["etd"].is_null(), "{grd}");
    let iss = row(flights, "ISS");
    assert_eq!(iss["cfr_issued"], true, "{iss}");
    assert_eq!(iss["wheels_up_proposed"], false, "{iss}");
    let lag = (at(&iss["etd"]) - locked).num_seconds().abs();
    assert!(
        lag <= 1,
        "timed from its CFR, not its proposed wheels-up: {iss}"
    );
    let air = row(flights, "AIR");
    assert_eq!(air["wheels_up_proposed"], false, "{air}");
    assert!(air["etd"].is_null(), "{air}");
}

/// `minutes` from now, on a whole minute: the precision a CFR ready time or a filed time carries.
fn whole_minute(minutes: i64) -> DateTime<Utc> {
    (Utc::now() + Duration::minutes(minutes))
        .with_second(0)
        .and_then(|t| t.with_nanosecond(0))
        .unwrap()
}

#[sqlx::test]
async fn a_cfr_with_a_ready_time_locks_it_and_meters_from_it(pool: PgPool) {
    let data = VatsimData {
        pilots: vec![parked("ISS", "")],
        ..Default::default()
    };
    let state = state_with(&pool, data).await;
    sqlx::query("insert into tmu.programs (icao, aar) values ('KDCA', 30)")
        .execute(&pool)
        .await
        .unwrap();
    let cookie = holder(&pool, &["tmu.cfr.assign", "tmu.program.read"]).await;
    let ready = whole_minute(60);
    let (status, body) = call(
        &state,
        Method::POST,
        "/api/v1/tmu/cfr",
        &[("cookie", &cookie), ("if-none-match", "*")],
        Some(json!({ "callsign": "ISS", "airport": "KDCA", "ready_time": ready })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        at(&body["wheels_up"]),
        ready,
        "the ready time is the wheels-up: {body}"
    );
    let stored: DateTime<Utc> =
        sqlx::query_scalar("select wheels_up from tmu.issued_cfrs where callsign = 'ISS'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored, ready);

    let (_, flow) = send_json(&state, Method::GET, "/api/v1/tmu/flow/KDCA", &cookie).await;
    let iss = row(&flow["flights"], "ISS");
    assert_eq!(at(&iss["cfr"]), ready, "{iss}");
    assert_eq!(iss["cfr_issued"], true, "{iss}");
    assert_eq!(iss["wheels_up_proposed"], false, "{iss}");
    assert_eq!(
        at(&iss["etd"]),
        ready,
        "timed from the runway at its ready time: {iss}"
    );
    assert_eq!(
        at(&iss["sta"]) - at(&iss["cfr"]),
        at(&iss["eta"]) - at(&iss["etd"]),
        "metered one airborne ETE after it: {iss}"
    );
}

#[sqlx::test]
async fn a_cfr_without_a_ready_time_locks_the_filed_departure_plus_ten(pool: PgPool) {
    let filed = whole_minute(40);
    let data = VatsimData {
        pilots: vec![parked("ISS", &filed.format("%H%M").to_string())],
        ..Default::default()
    };
    let state = state_with(&pool, data).await;
    let cookie = holder(&pool, &["tmu.cfr.assign"]).await;
    let (status, body) = call(
        &state,
        Method::POST,
        "/api/v1/tmu/cfr",
        &[("cookie", &cookie), ("if-none-match", "*")],
        Some(json!({ "callsign": "ISS", "airport": "KDCA" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        at(&body["wheels_up"]),
        filed + Duration::minutes(10),
        "the filed departure plus 10 min, not now plus 10: {body}"
    );
}

async fn issue_cfr(pool: &PgPool, callsign: &str, wheels_up: DateTime<Utc>) {
    sqlx::query(
        "insert into tmu.issued_cfrs (callsign, airport, wheels_up) values ($1, 'KDCA', $2)",
    )
    .bind(callsign)
    .bind(wheels_up)
    .execute(pool)
    .await
    .unwrap();
}

/// The gap between two ETAs, in seconds.
fn gap(later: &Value, earlier: &Value) -> i64 {
    (at(later) - at(earlier)).num_seconds()
}

/// #814 AC2: a flight holding a lock from another scheduler (a CFR, a GDP slot, another FCA's
/// release) is timed from that lock on the FCA traffic route, unmarked. Twins parked at one spot
/// with one plan share an airborne ETE, so their ETAs differ by exactly lock - (now + 10 min).
#[sqlx::test]
async fn fca_traffic_times_a_flight_from_a_lock_held_elsewhere(pool: PgPool) {
    let data = VatsimData {
        pilots: ["UNL", "CFR", "GDP", "OTH"].map(|cs| parked(cs, "")).into(),
        ..Default::default()
    };
    let state = state_with(&pool, data).await;
    let id = corridor_fca(&pool).await;
    let other = corridor_fca(&pool).await;
    let now = Utc::now();
    issue_cfr(&pool, "CFR", now + Duration::minutes(45)).await;
    let gdp: String = sqlx::query_scalar(
        "insert into tmu.gdp (airport, aar, start_time, end_time, status) \
         values ('KDCA', 30, '0000', '2359', 'published') returning id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "insert into tmu.gdp_slot (gdp_id, callsign, original_eta, cta, edct) \
         values ($1, 'GDP', $2, $2, $2)",
    )
    .bind(&gdp)
    .bind(now + Duration::minutes(60))
    .execute(&pool)
    .await
    .unwrap();
    let edct = (now + Duration::minutes(50)).timestamp_millis();
    sqlx::query(
        "insert into flow.fca_release (fca_id, callsign, cta_ms, edct_ms) values ($1, 'OTH', $2, $3)",
    )
    .bind(&other)
    .bind(edct + 30 * 60_000)
    .bind(edct)
    .execute(&pool)
    .await
    .unwrap();

    let (status, board) = send_json(
        &state,
        Method::GET,
        &format!("/api/v1/flow/fcas/{id}/traffic"),
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{board}");
    let unl = row(&board, "UNL");
    assert_eq!(unl["wheels_up_proposed"], true, "positive control: {unl}");
    for (cs, lock_min) in [("CFR", 45), ("GDP", 60), ("OTH", 50)] {
        let f = row(&board, cs);
        assert_eq!(f["wheels_up_proposed"], false, "{cs}: {f}");
        assert_eq!(f["released"], false, "{cs} holds no release here: {f}");
        let shift = gap(&f["eta"], &unl["eta"]);
        let want = (lock_min - 10) * 60;
        assert!(
            (shift - want).abs() <= 2,
            "{cs}: ETA {shift} s after the unlocked twin, the lock gives {want} s: {f}"
        );
    }
}

/// #814 AC2: the route popup's per-fix table and the IDST advisory EDCT time a CFR-held flight from
/// its CFR, as the FCA board does.
#[sqlx::test]
async fn the_fix_table_and_the_idst_advisory_time_from_a_cfr(pool: PgPool) {
    let data = VatsimData {
        pilots: vec![parked("UNL", ""), parked("CFR", "")],
        ..Default::default()
    };
    let state = state_with(&pool, data).await;
    let id = corridor_fca(&pool).await;
    let lock = Utc::now() + Duration::minutes(45);
    issue_cfr(&pool, "CFR", lock).await;

    let last_fix = |route: &Value| route["fixes"].as_array().unwrap().last().unwrap().clone();
    let (status, unl) = send_json(&state, Method::GET, "/api/v1/flow/aircraft/UNL/route", "").await;
    assert_eq!(status, StatusCode::OK, "{unl}");
    let (_, cfr) = send_json(&state, Method::GET, "/api/v1/flow/aircraft/CFR/route", "").await;
    let (unl, cfr) = (last_fix(&unl), last_fix(&cfr));
    assert_eq!(cfr["name"], "KDCA", "{cfr}");
    let shift = gap(&cfr["eta"], &unl["eta"]);
    assert!(
        (shift - 35 * 60).abs() <= 2,
        "the CFR moves the arrival 35 min past now + 10: {shift} s"
    );

    let (_, board) = send_json(
        &state,
        Method::GET,
        &format!("/api/v1/flow/fcas/{id}/traffic"),
        "",
    )
    .await;
    let ete = at(&row(&board, "CFR")["eta"]) - lock;
    let reader = holder(&pool, &["flow.fca.read"]).await;
    let (status, idst) = send_json(
        &state,
        Method::GET,
        "/api/v1/flow/idst?airports=KJFK",
        &reader,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{idst}");
    let advisory = row(&idst["unscheduled"], "CFR");
    let want = at(&advisory["cross_time"]) - ete;
    let off = (at(&advisory["edct"]) - want).num_seconds().abs();
    assert!(
        off <= 2,
        "the advisory EDCT is the CTA less the ETE from the CFR, off by {off} s: {advisory}"
    );
}

/// #814 AC2 and AC6: clearing a flight's release returns it timed from what it still holds: its
/// proposed wheels-up, marked, when nothing else locks it; its CFR, unmarked, when one does.
#[sqlx::test]
async fn clearing_a_release_times_the_flight_from_what_it_still_holds(pool: PgPool) {
    let data = VatsimData {
        pilots: vec![parked("FREE", ""), parked("CFR", "")],
        ..Default::default()
    };
    let state = state_with(&pool, data).await;
    let id = corridor_fca(&pool).await;
    let lock = Utc::now() + Duration::minutes(45);
    issue_cfr(&pool, "CFR", lock).await;
    let user = seed_user(&pool).await;
    grant(&pool, &user, "flow.fca.update", Some("ZDC")).await;
    let cookie = session_cookie(&pool, &user).await;
    let release = |cs: &str| format!("/api/v1/flow/fcas/{id}/release/{cs}");
    // FREE by RDY; CFR by SET an hour out, later than its CFR, so the cleared release is the lock
    // the flight was timed from and clearing it must hand the CFR back.
    let later = whole_minute(60).format("%H%M").to_string();
    for (cs, body) in [("FREE", json!({})), ("CFR", json!({ "ready": later }))] {
        let (status, body) = call(
            &state,
            Method::POST,
            &release(cs),
            &[("cookie", &cookie), ("if-none-match", "*")],
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{cs}: {body}");
        assert_eq!(row(&body, cs)["released"], true, "{cs}: {body}");
    }

    let (status, cleared) = call(
        &state,
        Method::DELETE,
        &release("FREE"),
        &[("cookie", &cookie)],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    let free = row(&cleared, "FREE");
    assert_eq!(free["released"], false, "{free}");
    assert_eq!(free["wheels_up_proposed"], true, "{free}");
    assert!(
        free["release_version"].is_null(),
        "the cleared release's version is gone with it: {free}"
    );

    let (status, cleared) = call(
        &state,
        Method::DELETE,
        &release("CFR"),
        &[("cookie", &cookie)],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    let (free, cfr) = (row(&cleared, "FREE"), row(&cleared, "CFR"));
    assert_eq!(cfr["released"], false, "{cfr}");
    assert_eq!(cfr["wheels_up_proposed"], false, "{cfr}");
    let shift = gap(&cfr["eta"], &free["eta"]);
    assert!(
        (shift - 35 * 60).abs() <= 2,
        "timed from its CFR, not the cleared release an hour out: {shift} s"
    );
}

/// #814: RDY and SET never release a flight before a lock it holds elsewhere. Held to a CFR 45 min
/// out, RDY (ready now) and SET (ready in 20 min) both release it at or after the CFR, and the board
/// times it from the release that results. Its own release here floors nothing: a SET an hour out,
/// then RDY, comes back near now.
#[sqlx::test]
async fn rdy_and_set_release_no_sooner_than_a_lock_held_elsewhere(pool: PgPool) {
    let data = VatsimData {
        pilots: vec![parked("CFR", ""), parked("FREE", "")],
        ..Default::default()
    };
    let state = state_with(&pool, data).await;
    let id = corridor_fca(&pool).await;
    let lock = whole_minute(45);
    issue_cfr(&pool, "CFR", lock).await;
    let user = seed_user(&pool).await;
    grant(&pool, &user, "flow.fca.update", Some("ZDC")).await;
    let cookie = session_cookie(&pool, &user).await;
    let release = |cs: &str| format!("/api/v1/flow/fcas/{id}/release/{cs}");
    let hhmm = |t: DateTime<Utc>| t.format("%H%M").to_string();

    for (how, body) in [
        ("RDY", json!({})),
        ("SET", json!({ "ready": hhmm(whole_minute(20)) })),
    ] {
        let (status, board) = call(
            &state,
            Method::POST,
            &release("CFR"),
            &[("cookie", &cookie)],
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{how}: {board}");
        let f = row(&board, "CFR");
        let early = (lock - at(&f["edct"])).num_seconds();
        assert!(
            early <= 0,
            "{how} released it {early} s before its CFR: {f}"
        );
        assert_eq!(
            f["eta"], f["cross_time"],
            "{how}: timed from the release: {f}"
        );
    }

    let later = whole_minute(60);
    let (status, set) = call(
        &state,
        Method::POST,
        &release("FREE"),
        &[("cookie", &cookie)],
        Some(json!({ "ready": hhmm(later) })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{set}");
    assert!(
        at(&row(&set, "FREE")["edct"]) >= later,
        "positive control: SET an hour out"
    );
    let (status, rdy) = call(
        &state,
        Method::POST,
        &release("FREE"),
        &[("cookie", &cookie)],
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{rdy}");
    let edct = at(&row(&rdy, "FREE")["edct"]);
    assert!(
        edct < Utc::now() + Duration::minutes(10),
        "RDY is not held to its own SET an hour out: {edct}"
    );
}

/// #814 AC4: a prefile gets no proposed wheels-up, so airport flow and the departures list leave
/// it out, locked or not, beside a connected pilot on the same plan that each lists. Its route popup
/// draws the route with no per-fix table unless it holds a lock.
#[sqlx::test]
async fn a_prefile_leaves_flow_and_departures_and_gets_a_fix_table_only_when_locked(pool: PgPool) {
    let prefile = |cs: &str| Prefile {
        callsign: cs.into(),
        flight_plan: Some(plan("")),
        ..Default::default()
    };
    let data = VatsimData {
        pilots: vec![parked("GRD", "")],
        prefiles: vec![prefile("PRE"), prefile("PRL")],
        ..Default::default()
    };
    let state = state_with(&pool, data).await;
    sqlx::query("insert into tmu.programs (icao, aar) values ('KDCA', 30)")
        .execute(&pool)
        .await
        .unwrap();
    issue_cfr(&pool, "PRL", Utc::now() + Duration::minutes(45)).await;
    let reader = holder(&pool, &["tmu.program.read"]).await;
    let listed = |body: &Value, key: &str| -> Vec<String> {
        body[key]
            .as_array()
            .unwrap_or_else(|| panic!("{key} missing: {body}"))
            .iter()
            .map(|f| f["callsign"].as_str().unwrap().to_string())
            .collect()
    };

    let (status, flow) = send_json(&state, Method::GET, "/api/v1/tmu/flow/KDCA", &reader).await;
    assert_eq!(status, StatusCode::OK, "{flow}");
    assert_eq!(listed(&flow, "flights"), ["GRD"], "{flow}");
    assert_eq!(flow["inbound"], 1, "{flow}");
    assert!(
        flow.get("proposed").is_none(),
        "Flow.proposed is gone: {flow}"
    );

    let (status, deps) =
        send_json(&state, Method::GET, "/api/v1/tmu/departures/KJFK", &reader).await;
    assert_eq!(status, StatusCode::OK, "{deps}");
    assert_eq!(listed(&deps, "departures"), ["GRD"], "{deps}");

    let fixes = |route: &Value| route["fixes"].as_array().map_or(0, Vec::len);
    let (status, unlocked) =
        send_json(&state, Method::GET, "/api/v1/flow/aircraft/PRE/route", "").await;
    assert_eq!(status, StatusCode::OK, "{unlocked}");
    assert_eq!(
        unlocked["callsign"], "PRE",
        "the route is still served: {unlocked}"
    );
    assert_eq!(fixes(&unlocked), 0, "no lock, no table: {unlocked}");
    let (status, locked) =
        send_json(&state, Method::GET, "/api/v1/flow/aircraft/PRL/route", "").await;
    assert_eq!(status, StatusCode::OK, "{locked}");
    assert!(
        fixes(&locked) > 0,
        "positive control: locked, it has a table: {locked}"
    );
}

/// One release on `fca_id` for `callsign`, wheels-up at `edct`, crossing 30 min later.
async fn release_on(pool: &PgPool, fca_id: &str, callsign: &str, edct: DateTime<Utc>) {
    let edct = edct.timestamp_millis();
    sqlx::query(
        "insert into flow.fca_release (fca_id, callsign, cta_ms, edct_ms) values ($1, $2, $3, $4)",
    )
    .bind(fca_id)
    .bind(callsign)
    .bind(edct + 30 * 60_000)
    .bind(edct)
    .execute(pool)
    .await
    .unwrap();
}

async fn program_at_kdca(pool: &PgPool, aar: i32) {
    sqlx::query("insert into tmu.programs (icao, aar) values ('KDCA', $1)")
        .bind(aar)
        .execute(pool)
        .await
        .unwrap();
}

/// #814 AC4: a CFR issued with no ready time at a rate-program field locks the program's metered
/// CFR, not the flight's proposed wheels-up. AIR lands first and the AAR of 1 holds ISS an hour
/// behind it, so the two differ.
#[sqlx::test]
async fn a_cfr_without_a_ready_time_locks_the_programs_metered_cfr(pool: PgPool) {
    let data = VatsimData {
        pilots: vec![airborne("AIR"), parked("ISS", "")],
        ..Default::default()
    };
    let state = state_with(&pool, data).await;
    program_at_kdca(&pool, 1).await;
    let cookie = holder(&pool, &["tmu.cfr.assign", "tmu.program.read"]).await;
    let (_, flow) = send_json(&state, Method::GET, "/api/v1/tmu/flow/KDCA", &cookie).await;
    let iss = row(&flow["flights"], "ISS");
    let (cfr, etd) = (at(&iss["cfr"]), at(&iss["etd"]));
    assert!(
        cfr - etd >= Duration::minutes(10),
        "positive control: metering holds ISS past its proposed wheels-up: {iss}"
    );

    let (status, body) = call(
        &state,
        Method::POST,
        "/api/v1/tmu/cfr",
        &[("cookie", &cookie), ("if-none-match", "*")],
        Some(json!({ "callsign": "ISS", "airport": "KDCA" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let locked = at(&body["wheels_up"]);
    assert!(
        (locked - cfr).num_seconds().abs() <= 2,
        "the metered CFR {cfr}, not the proposed wheels-up {etd}: {body}"
    );
}

/// #814 AC4: with no program at the field, a CFR issued with no ready time locks the wheels-up the
/// flight already holds (here another FCA's release), not its proposed one.
#[sqlx::test]
async fn a_cfr_without_a_ready_time_locks_the_lock_the_flight_holds(pool: PgPool) {
    let data = VatsimData {
        pilots: vec![parked("ISS", "")],
        ..Default::default()
    };
    let state = state_with(&pool, data).await;
    let fca = corridor_fca(&pool).await;
    let lock = whole_minute(50);
    release_on(&pool, &fca, "ISS", lock).await;
    let cookie = holder(&pool, &["tmu.cfr.assign"]).await;
    let (status, body) = call(
        &state,
        Method::POST,
        "/api/v1/tmu/cfr",
        &[("cookie", &cookie), ("if-none-match", "*")],
        Some(json!({ "callsign": "ISS", "airport": "KDCA" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        at(&body["wheels_up"]),
        lock,
        "its release, not now + 10: {body}"
    );
}

/// #814 decision 19: SET with a ready time later than a lock held elsewhere releases at the ready
/// time; the lock floors the search, it does not replace a later ready time.
#[sqlx::test]
async fn a_set_later_than_a_lock_held_elsewhere_releases_at_the_ready_time(pool: PgPool) {
    let data = VatsimData {
        pilots: vec![parked("CFR", "")],
        ..Default::default()
    };
    let state = state_with(&pool, data).await;
    let id = corridor_fca(&pool).await;
    issue_cfr(&pool, "CFR", whole_minute(30)).await;
    let cookie = holder(&pool, &["flow.fca.update"]).await;
    let ready = whole_minute(60);
    let (status, board) = call(
        &state,
        Method::POST,
        &format!("/api/v1/flow/fcas/{id}/release/CFR"),
        &[("cookie", &cookie)],
        Some(json!({ "ready": ready.format("%H%M").to_string() })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{board}");
    assert_eq!(at(&row(&board, "CFR")["edct"]), ready, "{board}");
}

/// #814: the release response is the board the web caches. A SET an hour out comes back with no
/// delay, timed from the release; and another flight held to a CFR comes back timed from its CFR
/// and unmarked, as the next read shows it.
#[sqlx::test]
async fn the_release_response_is_timed_like_the_board(pool: PgPool) {
    let data = VatsimData {
        pilots: vec![parked("SET", ""), parked("HLD", "")],
        ..Default::default()
    };
    let state = state_with(&pool, data).await;
    let id = corridor_fca(&pool).await;
    let hold = whole_minute(45);
    issue_cfr(&pool, "HLD", hold).await;
    let cookie = holder(&pool, &["flow.fca.update"]).await;
    let ready = whole_minute(60);
    let (status, board) = call(
        &state,
        Method::POST,
        &format!("/api/v1/flow/fcas/{id}/release/SET"),
        &[("cookie", &cookie)],
        Some(json!({ "ready": ready.format("%H%M").to_string() })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{board}");
    let set = row(&board, "SET");
    assert_eq!(
        set["delay_sec"], 0,
        "a fresh release carries no delay: {set}"
    );
    assert_eq!(set["eta"], set["cross_time"], "{set}");
    let hld = row(&board, "HLD");
    assert_eq!(hld["wheels_up_proposed"], false, "{hld}");
    assert!(
        at(&hld["eta"]) > hold,
        "timed from its CFR {hold}, not now + 10: {hld}"
    );
    let (_, reread) = send_json(
        &state,
        Method::GET,
        &format!("/api/v1/flow/fcas/{id}/traffic"),
        "",
    )
    .await;
    let lag = gap(&row(&reread, "HLD")["eta"], &hld["eta"]).abs();
    assert!(lag <= 2, "the response matches the next read: {lag} s");
}

/// #814 on a board whose own release is not in the lock map (a disabled FCA): a released flight is
/// still timed from its release here, the later of it and an earlier CFR it holds, and unmarked.
#[sqlx::test]
async fn a_disabled_fcas_board_times_a_flight_from_its_release_there(pool: PgPool) {
    let data = VatsimData {
        pilots: vec![parked("REL", ""), parked("FREE", "")],
        ..Default::default()
    };
    let state = state_with(&pool, data).await;
    let proposed = Utc::now() + Duration::minutes(10);
    let id = corridor_fca(&pool).await;
    sqlx::query("update flow.fca set enabled = false where id = $1")
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
    let release = whole_minute(50);
    release_on(&pool, &id, "REL", release).await;
    issue_cfr(&pool, "REL", whole_minute(20)).await;
    let (status, board) = send_json(
        &state,
        Method::GET,
        &format!("/api/v1/flow/fcas/{id}/traffic"),
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{board}");
    let rel = row(&board, "REL");
    assert_eq!(rel["released"], true, "{rel}");
    assert_eq!(rel["wheels_up_proposed"], false, "{rel}");
    assert_eq!(at(&rel["edct"]), release, "{rel}");
    let shift = gap(&rel["eta"], &row(&board, "FREE")["eta"]);
    let want = (release - proposed).num_seconds();
    assert!(
        (shift - want).abs() <= 3,
        "timed from the release here, {want} s after an unlocked twin, not the CFR: {shift} s"
    );
}

/// #814: `GET /api/v1/tmu/demand` counts a flight from its lock. Both depart KBWI for KDCA, well
/// inside the hour from their proposed wheels-up; the one held to a CFR two hours out is not. A field
/// only a prefile flies to has no row.
#[sqlx::test]
async fn airport_demand_counts_a_locked_flight_from_its_lock(pool: PgPool) {
    let near = |cs: &str| Pilot {
        callsign: cs.into(),
        latitude: 39.18,
        longitude: -76.67,
        flight_plan: Some(FlightPlan {
            departure: "KBWI".into(),
            arrival: "KDCA".into(),
            route: "DCT".into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let state = test_state(pool.clone(), HashMap::new());
    {
        let mut feed = state.feed.write().await;
        feed.airports = Arc::new(HashMap::from([
            ("KBWI".to_string(), Airport::at(39.18, -76.67)),
            ("KDCA".to_string(), Airport::at(38.85, -77.04)),
        ]));
        feed.snapshot = Some(Arc::new(crate::feed::Snapshot::of(VatsimData {
            pilots: vec![near("UNL"), near("LCK")],
            prefiles: vec![Prefile {
                callsign: "PRE".into(),
                flight_plan: Some(FlightPlan {
                    departure: "KBWI".into(),
                    arrival: "KIAD".into(),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        })));
    }
    program_at_kdca(&pool, 30).await;
    sqlx::query("insert into tmu.programs (icao, aar) values ('KIAD', 30)")
        .execute(&pool)
        .await
        .unwrap();
    issue_cfr(&pool, "LCK", Utc::now() + Duration::hours(2)).await;
    let reader = holder(&pool, &["tmu.program.read"]).await;
    let (status, rows) = send_json(&state, Method::GET, "/api/v1/tmu/demand", &reader).await;
    assert_eq!(status, StatusCode::OK, "{rows}");
    let kdca = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["icao"] == "KDCA")
        .unwrap_or_else(|| panic!("KDCA missing: {rows}"));
    assert_eq!(kdca["demand_60min"], 1, "UNL only: {kdca}");
    assert!(
        rows.as_array().unwrap().iter().all(|r| r["icao"] != "KIAD"),
        "an airport only a prefile flies to is not ranked: {rows}"
    );
}

/// #814: a CFR issued with a ready time slots behind another flight timed from its lock. LCK holds
/// an FCA release 30 min out; ISS, ready then too, takes the next slot an AAR of 1 allows, an hour
/// later, not the one LCK's proposed wheels-up would have left open.
#[sqlx::test]
async fn a_cfr_with_a_ready_time_slots_behind_a_locked_arrival(pool: PgPool) {
    let ready = whole_minute(30);
    let data = VatsimData {
        pilots: vec![
            parked("LCK", ""),
            parked("ISS", &ready.format("%H%M").to_string()),
        ],
        ..Default::default()
    };
    let state = state_with(&pool, data).await;
    program_at_kdca(&pool, 1).await;
    let fca = corridor_fca(&pool).await;
    release_on(&pool, &fca, "LCK", ready).await;
    let cookie = holder(&pool, &["tmu.cfr.assign", "tmu.program.read"]).await;
    let (status, body) = call(
        &state,
        Method::POST,
        "/api/v1/tmu/cfr",
        &[("cookie", &cookie), ("if-none-match", "*")],
        Some(json!({ "callsign": "ISS", "airport": "KDCA", "ready_time": ready })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let behind = at(&body["wheels_up"]) - ready;
    assert!(
        behind >= Duration::minutes(55),
        "an hour behind LCK's locked arrival, got {} min: {body}",
        behind.num_minutes()
    );
}

/// The IDST runway column reads the stored departure-runway assignment (#511), kept by #814.
#[sqlx::test]
async fn idst_shows_the_assigned_departure_runway(pool: PgPool) {
    let data = VatsimData {
        pilots: vec![parked("GRD", ""), parked("NONE", "")],
        ..Default::default()
    };
    let state = state_with(&pool, data).await;
    corridor_fca(&pool).await;
    sqlx::query(
        "insert into flow.departure_runway_assignment (icao, callsign, runway, source) values ('KJFK', 'GRD', '31L', 'config')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let reader = holder(&pool, &["flow.fca.read"]).await;
    let (status, idst) = send_json(
        &state,
        Method::GET,
        "/api/v1/flow/idst?airports=KJFK",
        &reader,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{idst}");
    let grd = row(&idst["unscheduled"], "GRD");
    assert_eq!(grd["runway"], "31L", "{grd}");
    assert_eq!(grd["runway_source"], "config", "{grd}");
    let none = row(&idst["unscheduled"], "NONE");
    assert!(
        none["runway"].is_null(),
        "positive control: no row, no runway: {none}"
    );
}

/// #814 decision 26: a CFR for a callsign that is only a prefile, which airport flow no longer
/// lists, keeps the fallback for any unlisted flight: now with no ready time, the ready time
/// unmetered with one.
#[sqlx::test]
async fn a_cfr_for_a_prefile_only_callsign_takes_the_unlisted_fallback(pool: PgPool) {
    let data = VatsimData {
        pilots: vec![parked("GRD", "")],
        prefiles: vec![Prefile {
            callsign: "PRE".into(),
            flight_plan: Some(plan("")),
            ..Default::default()
        }],
        ..Default::default()
    };
    let state = state_with(&pool, data).await;
    program_at_kdca(&pool, 30).await;
    let cookie = holder(&pool, &["tmu.cfr.assign", "tmu.program.read"]).await;
    let before = Utc::now();
    let (status, body) = call(
        &state,
        Method::POST,
        "/api/v1/tmu/cfr",
        &[("cookie", &cookie), ("if-none-match", "*")],
        Some(json!({ "callsign": "PRE", "airport": "KDCA" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let locked = at(&body["wheels_up"]);
    assert!(
        locked >= before && locked <= Utc::now(),
        "now, with no ready time: {body}"
    );
    let ready = whole_minute(40);
    let (status, body) = call(
        &state,
        Method::POST,
        "/api/v1/tmu/cfr",
        &[("cookie", &cookie)],
        Some(json!({ "callsign": "PRE", "airport": "KDCA", "ready_time": ready })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        at(&body["wheels_up"]),
        ready,
        "the ready time, unmetered: {body}"
    );
}

/// #814 decision 40: the lock-held-elsewhere floor on RDY applies to a ground flight only. AIR, in
/// the air but still holding a CFR 45 min out, is released at the crossing it can make; GRD, parked
/// and held to the same CFR, is released no sooner than that CFR (the positive control).
#[sqlx::test]
async fn rdy_does_not_floor_an_airborne_flight_at_a_lock_it_still_holds(pool: PgPool) {
    let data = VatsimData {
        pilots: vec![airborne("AIR"), parked("GRD", "")],
        ..Default::default()
    };
    let state = state_with(&pool, data).await;
    let id = corridor_fca(&pool).await;
    let lock = whole_minute(45);
    issue_cfr(&pool, "AIR", lock).await;
    issue_cfr(&pool, "GRD", lock).await;
    let traffic = format!("/api/v1/flow/fcas/{id}/traffic");
    let (_, before) = send_json(&state, Method::GET, &traffic, "").await;
    let air_eta = at(&row(&before, "AIR")["eta"]);
    assert!(
        air_eta < lock,
        "positive control: AIR crosses before its CFR: {before}"
    );
    let cookie = holder(&pool, &["flow.fca.update"]).await;
    let headers = [("cookie", cookie.as_str())];
    let release = |cs: &str| format!("/api/v1/flow/fcas/{id}/release/{cs}");
    let (status, board) = call(
        &state,
        Method::POST,
        &release("AIR"),
        &headers,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{board}");
    let cross = at(&row(&board, "AIR")["cross_time"]);
    let late = (cross - air_eta).num_seconds();
    assert!(
        (0..=180).contains(&late),
        "released at the crossing it can make, {late} s after its ETA, not after its CFR: {board}"
    );
    let (status, board) = call(
        &state,
        Method::POST,
        &release("GRD"),
        &headers,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{board}");
    let early = (lock - at(&row(&board, "GRD")["edct"])).num_seconds();
    assert!(
        early <= 0,
        "a ground flight is floored at its CFR: {early} s early: {board}"
    );
}
