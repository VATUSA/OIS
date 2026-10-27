//! The FCA freeze horizon's stored STAs (#817): what the metered read writes, what every later read
//! serves, and what prunes or clears them.
//!
//! The router tests use the corridor feed from `flow_prefile_candidate_tests`: an FCA between WHITE
//! and SIE on the KJFK->KDCA route, and an aircraft climbing out of KJFK that crosses it airborne
//! inside the default 30 min horizon.

use std::collections::HashMap;
use std::sync::Arc;

use axum::http::{Method, StatusCode};
use chrono::Utc;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::feed::airports::Airport;
use crate::feed::vatsim::{FlightPlan, Pilot, VatsimData};
use crate::repos::flow as flow_repo;
use crate::scope_test_support::{grant, seed_user, send, send_json, session_cookie, test_state};
use crate::state::AppState;

fn plan() -> FlightPlan {
    FlightPlan {
        departure: "KJFK".into(),
        arrival: "KDCA".into(),
        route: "RBV WHITE SIE".into(),
        altitude: "35000".into(),
        cruise_tas: "440".into(),
        ..Default::default()
    }
}

/// Climbing out of KJFK, short of the corridor line: it crosses the FCA airborne, minutes out.
fn airborne(callsign: &str) -> Pilot {
    Pilot {
        callsign: callsign.into(),
        latitude: 40.45,
        longitude: -74.1,
        altitude: 8_000,
        groundspeed: 300,
        heading: 230,
        flight_plan: Some(plan()),
        ..Default::default()
    }
}

/// Parked at KJFK: a ground departure, which the horizon never freezes.
fn parked(callsign: &str) -> Pilot {
    Pilot {
        callsign: callsign.into(),
        latitude: 40.64,
        longitude: -73.78,
        altitude: 13,
        groundspeed: 0,
        heading: 310,
        flight_plan: Some(plan()),
        ..Default::default()
    }
}

async fn state_with(pool: PgPool, pilots: Vec<Pilot>) -> AppState {
    let state = test_state(pool, HashMap::new());
    {
        let mut feed = state.feed.write().await;
        feed.airports = Arc::new(HashMap::from([
            ("KJFK".to_string(), Airport::at(40.64, -73.78)),
            ("KDCA".to_string(), Airport::at(38.85, -77.04)),
        ]));
        feed.snapshot = Some(Arc::new(crate::feed::Snapshot::of(VatsimData {
            pilots,
            ..Default::default()
        })));
    }
    state
}

/// An enabled rate-30 FCA on the corridor with the given freeze horizon, and its id.
async fn corridor_fca(pool: &PgPool, freeze_min: i32) -> String {
    sqlx::query_scalar(
        "insert into flow.fca (name, color, artcc, points, dir, mode, rate, mit, enabled, freeze_min) \
         values ('FRZ', '#fff', 'ZDC', $1, 'any', 'rate', 30, 0, true, $2) returning id",
    )
    .bind(json!([[39.5, -75.6], [39.5, -74.0]]))
    .bind(freeze_min)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn held_rows(pool: &PgPool, fca_id: &str) -> Vec<(String, i64)> {
    sqlx::query_as("select callsign, sta_ms from flow.fca_held_sta where fca_id = $1 order by 1")
        .bind(fca_id)
        .fetch_all(pool)
        .await
        .unwrap()
}

async fn seed_held(pool: &PgPool, fca_id: &str, callsign: &str, sta_ms: i64) {
    sqlx::query("insert into flow.fca_held_sta (fca_id, callsign, sta_ms) values ($1, $2, $3)")
        .bind(fca_id)
        .bind(callsign)
        .bind(sta_ms)
        .execute(pool)
        .await
        .unwrap();
}

async fn traffic(state: &AppState, fca_id: &str) -> Vec<Value> {
    let (status, body) = send_json(
        state,
        Method::GET,
        &format!("/api/v1/flow/fcas/{fca_id}/traffic"),
        "",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    body.as_array().unwrap().clone()
}

fn row<'a>(rows: &'a [Value], callsign: &str) -> &'a Value {
    rows.iter()
        .find(|f| f["callsign"] == callsign)
        .unwrap_or_else(|| panic!("{callsign} is not on the board: {rows:?}"))
}

fn ms(v: &Value) -> i64 {
    chrono::DateTime::parse_from_rfc3339(v.as_str().unwrap())
        .unwrap()
        .timestamp_millis()
}

#[sqlx::test]
async fn the_traffic_read_holds_an_airborne_flights_sta_and_not_a_ground_one(pool: PgPool) {
    let state = state_with(pool.clone(), vec![airborne("AIR1"), parked("GND1")]).await;
    let id = corridor_fca(&pool, 30).await;

    let rows = traffic(&state, &id).await;
    let air = row(&rows, "AIR1");
    assert_eq!(air["sta_frozen"], true, "minutes out is inside 30 min");
    assert_eq!(row(&rows, "GND1")["sta_frozen"], false);
    assert_eq!(
        held_rows(&pool, &id).await,
        vec![("AIR1".to_string(), ms(&air["cross_time"]))],
        "the read stores the airborne flight's STA, and only that one"
    );
}

#[sqlx::test]
async fn a_held_sta_is_served_on_the_next_read_while_the_eta_differs(pool: PgPool) {
    let state = state_with(pool.clone(), vec![airborne("AIR1")]).await;
    let id = corridor_fca(&pool, 30).await;
    let eta = ms(&row(&traffic(&state, &id).await, "AIR1")["eta"]);
    // Replace the stored STA with one 5 min after the ETA, as if the ETA had moved since.
    sqlx::query("delete from flow.fca_held_sta")
        .execute(&pool)
        .await
        .unwrap();
    seed_held(&pool, &id, "AIR1", eta + 300_000).await;

    // The handler reads the clock, so the ETA moves by a few ms between requests: compare each
    // response against its own ETA.
    let rows = traffic(&state, &id).await;
    let air = row(&rows, "AIR1");
    let eta_now = ms(&air["eta"]);
    assert!(
        (eta_now - eta).abs() < 5_000,
        "the fixture's ETA is stable to a few seconds"
    );
    assert_eq!(
        ms(&air["cross_time"]),
        eta + 300_000,
        "the held STA, not the ETA"
    );
    assert_eq!(air["delay_sec"], (eta + 300_000 - eta_now) / 1000);
    assert_eq!(air["late_sec"], 0);
    assert_eq!(air["sta_frozen"], true);
}

#[sqlx::test]
async fn a_late_held_flight_crosses_at_its_eta_and_shows_lateness(pool: PgPool) {
    let state = state_with(pool.clone(), vec![airborne("AIR1")]).await;
    let id = corridor_fca(&pool, 30).await;
    let eta = ms(&row(&traffic(&state, &id).await, "AIR1")["eta"]);
    sqlx::query("delete from flow.fca_held_sta")
        .execute(&pool)
        .await
        .unwrap();
    seed_held(&pool, &id, "AIR1", eta - 120_000).await;

    let rows = traffic(&state, &id).await;
    let air = row(&rows, "AIR1");
    let eta_now = ms(&air["eta"]);
    assert!(
        (eta_now - eta).abs() < 5_000,
        "the fixture's ETA is stable to a few seconds"
    );
    assert_eq!(
        ms(&air["cross_time"]),
        eta_now,
        "a late flight crosses at its ETA"
    );
    assert_eq!(air["late_sec"], (eta_now - (eta - 120_000)) / 1000);
    assert_eq!(air["delay_sec"], 0);
}

#[sqlx::test]
async fn freezing_off_stores_nothing(pool: PgPool) {
    let state = state_with(pool.clone(), vec![airborne("AIR1")]).await;
    let off = corridor_fca(&pool, 0).await;
    let on = corridor_fca(&pool, 30).await;

    assert_eq!(
        row(&traffic(&state, &off).await, "AIR1")["sta_frozen"],
        false
    );
    assert!(held_rows(&pool, &off).await.is_empty());
    // Positive control: the same flight on an FCA with a horizon is held.
    traffic(&state, &on).await;
    assert_eq!(held_rows(&pool, &on).await.len(), 1);
}

#[sqlx::test]
async fn a_stale_held_row_is_ignored_then_replaced(pool: PgPool) {
    let state = state_with(pool.clone(), vec![airborne("AIR1")]).await;
    let id = corridor_fca(&pool, 30).await;
    let stale = Utc::now().timestamp_millis() - 2 * flow_repo::HELD_STA_RETAIN_MS;
    seed_held(&pool, &id, "AIR1", stale).await;

    let rows = traffic(&state, &id).await;
    let air = row(&rows, "AIR1");
    assert_eq!(air["late_sec"], 0, "a stale row is not a held STA");
    assert_eq!(
        held_rows(&pool, &id).await,
        vec![("AIR1".to_string(), ms(&air["cross_time"]))],
        "the stale row is pruned and the current STA held"
    );
}

#[sqlx::test]
async fn hold_stas_keeps_the_first_sta_and_prunes_only_this_fcas_stale_rows(pool: PgPool) {
    let a = corridor_fca(&pool, 30).await;
    let b = corridor_fca(&pool, 30).await;
    let now = Utc::now();
    let stale = now.timestamp_millis() - flow_repo::HELD_STA_RETAIN_MS - 1;
    let fresh = now.timestamp_millis() - flow_repo::HELD_STA_RETAIN_MS + 60_000;
    seed_held(&pool, &a, "OLD1", stale).await;
    seed_held(&pool, &a, "KEEP", fresh).await;
    seed_held(&pool, &b, "OLD2", stale).await;
    seed_held(&pool, &b, "LIVE", fresh).await;

    let first = now.timestamp_millis() + 600_000;
    flow_repo::hold_stas(&pool, &a, &[("NEW1".into(), first)], now)
        .await
        .unwrap();
    flow_repo::hold_stas(&pool, &a, &[("NEW1".into(), first + 60_000)], now)
        .await
        .unwrap();

    assert_eq!(
        held_rows(&pool, &a).await,
        vec![("KEEP".to_string(), fresh), ("NEW1".to_string(), first)],
        "the first STA wins; only the stale row goes"
    );
    assert_eq!(
        held_rows(&pool, &b).await,
        vec![("LIVE".to_string(), fresh), ("OLD2".to_string(), stale)],
        "another FCA's rows are not pruned"
    );
    let listed = flow_repo::list_held_stas(&pool, &a, now).await.unwrap();
    assert_eq!(
        listed.get("KEEP"),
        Some(&fresh),
        "inside the retention window"
    );
    assert_eq!(
        listed.len(),
        2,
        "this FCA's two live rows, not the other FCA's: {listed:?}"
    );
}

#[sqlx::test]
async fn a_read_with_no_newly_frozen_flight_writes_nothing(pool: PgPool) {
    // AIR1 already holds an STA, so this read has nothing new to store and opens no write: the
    // stale row the next write would prune is still there afterwards.
    let state = state_with(pool.clone(), vec![airborne("AIR1")]).await;
    let id = corridor_fca(&pool, 30).await;
    let eta = ms(&row(&traffic(&state, &id).await, "AIR1")["eta"]);
    let stale = Utc::now().timestamp_millis() - flow_repo::HELD_STA_RETAIN_MS - 60_000;
    seed_held(&pool, &id, "GONE", stale).await;

    let rows = traffic(&state, &id).await;
    assert_eq!(
        row(&rows, "AIR1")["sta_frozen"],
        true,
        "positive control: AIR1 is held"
    );
    let held = held_rows(&pool, &id).await;
    assert_eq!(
        held.len(),
        2,
        "no write ran, so nothing was pruned: {held:?}"
    );
    assert_eq!(held[1], ("GONE".to_string(), stale));
    assert!(
        (held[0].1 - eta).abs() < 5_000,
        "AIR1's first STA is unchanged"
    );
}

#[sqlx::test]
async fn an_fca_edit_clears_its_held_stas_and_sets_the_horizon(pool: PgPool) {
    let state = state_with(pool.clone(), vec![]).await;
    let id = corridor_fca(&pool, 30).await;
    let other = corridor_fca(&pool, 30).await;
    seed_held(&pool, &id, "AIR1", Utc::now().timestamp_millis()).await;
    seed_held(&pool, &other, "AIR2", Utc::now().timestamp_millis()).await;
    let user = seed_user(&pool).await;
    grant(&pool, &user, "flow.fca.update", None).await;
    let cookie = session_cookie(&pool, &user).await;

    let status = send(
        &state,
        Method::PUT,
        &format!("/api/v1/flow/fcas/{id}"),
        &cookie,
        Some(json!({
            "name": "FRZ", "artcc": "ZDC", "points": [[39.5, -75.6], [39.5, -74.0]],
            "mode": "rate", "rate": 20, "freeze_min": 500
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        held_rows(&pool, &id).await.is_empty(),
        "the edit clears this FCA's rows"
    );
    assert_eq!(
        held_rows(&pool, &other).await.len(),
        1,
        "and no other FCA's"
    );
    let fca = flow_repo::get_fca(&pool, &id).await.unwrap().unwrap();
    assert_eq!(fca.freeze_min, 120, "the horizon is clamped to 120 min");
}

#[sqlx::test]
async fn a_new_fca_gets_the_default_horizon(pool: PgPool) {
    let user = seed_user(&pool).await;
    let by = crate::auth::principal::Attribution::user_only(&user);
    let req: crate::models::UpsertFcaRequest = serde_json::from_value(json!({
        "name": "NEW", "points": [[39.5, -75.6], [39.5, -74.0]]
    }))
    .unwrap();
    let id = flow_repo::create_fca(&pool, &req, &by).await.unwrap();
    let fca = flow_repo::get_fca(&pool, &id).await.unwrap().unwrap();
    assert_eq!(fca.freeze_min, crate::feed::fca::DEFAULT_FREEZE_MIN);

    let event_req: crate::models::UpsertFcaRequest = serde_json::from_value(json!({
        "name": "EVT", "points": [[39.5, -75.6], [39.5, -74.0]], "freeze_min": 12
    }))
    .unwrap();
    let event_id: i64 = sqlx::query_scalar(
        "insert into events.event (id, title, start_time, end_time) \
         values (817, 'E', now(), now() + interval '1 hour') returning id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let eid = flow_repo::create_event_fca(&pool, event_id, &event_req, &by)
        .await
        .unwrap();
    let efca = flow_repo::get_fca(&pool, &eid).await.unwrap().unwrap();
    assert_eq!(efca.freeze_min, 12, "the event path binds the horizon too");
    assert_eq!(efca.event_id, Some(event_id));
}
