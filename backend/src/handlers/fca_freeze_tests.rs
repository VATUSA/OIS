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

const MIN: i64 = 60_000;

#[test]
fn the_retention_window_sits_between_the_fixtures() {
    let retain = std::hint::black_box(flow_repo::HELD_STA_RETAIN_MS);
    assert!(
        59 * MIN < retain && retain < 61 * MIN,
        "the 59 and 61 min fixtures must straddle the retention, got {retain} ms"
    );
}

async fn updated_at(pool: &PgPool, fca_id: &str) -> chrono::DateTime<Utc> {
    flow_repo::get_fca(pool, fca_id)
        .await
        .unwrap()
        .unwrap()
        .updated_at
}

#[sqlx::test]
async fn sync_held_stas_keeps_the_first_sta_and_prunes_only_this_fcas_stale_rows(pool: PgPool) {
    let a = corridor_fca(&pool, 30).await;
    let b = corridor_fca(&pool, 30).await;
    let now = Utc::now();
    let stale = now.timestamp_millis() - 61 * MIN;
    let fresh = now.timestamp_millis() - 59 * MIN;
    seed_held(&pool, &a, "OLD1", stale).await;
    seed_held(&pool, &a, "KEEP", fresh).await;
    seed_held(&pool, &b, "OLD2", stale).await;
    seed_held(&pool, &b, "LIVE", fresh).await;

    let first = now.timestamp_millis() + 600_000;
    let at = updated_at(&pool, &a).await;
    for sta in [first, first + 60_000] {
        let store = [("NEW1".to_string(), sta)];
        let sync = flow_repo::HeldStaSync {
            store: &store,
            drop: &[],
        };
        flow_repo::sync_held_stas(&pool, &a, at, &sync, now)
            .await
            .unwrap();
    }

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
    let listed = flow_repo::list_held_stas(&pool, &[a.clone(), b.clone()], now)
        .await
        .unwrap();
    assert_eq!(
        listed[&a],
        HashMap::from([("KEEP".to_string(), fresh), ("NEW1".to_string(), first)]),
        "59 min past its STA is still held"
    );
    assert_eq!(
        listed[&b],
        HashMap::from([("LIVE".to_string(), fresh)]),
        "61 min past is not, and each FCA gets its own rows"
    );
    let only_a = flow_repo::list_held_stas(&pool, std::slice::from_ref(&a), now)
        .await
        .unwrap();
    let keys: Vec<&String> = only_a.keys().collect();
    assert_eq!(
        keys,
        vec![&a],
        "asking for one FCA reads only that FCA's rows"
    );
}

#[sqlx::test]
async fn sync_held_stas_drops_only_the_named_callsigns_of_this_fca(pool: PgPool) {
    let a = corridor_fca(&pool, 30).await;
    let b = corridor_fca(&pool, 30).await;
    let now = Utc::now();
    let sta = now.timestamp_millis() + 600_000;
    seed_held(&pool, &a, "GONE", sta).await;
    seed_held(&pool, &a, "KEEP", sta).await;
    seed_held(&pool, &b, "GONE", sta).await;

    let drop = ["GONE".to_string()];
    let sync = flow_repo::HeldStaSync {
        store: &[],
        drop: &drop,
    };
    flow_repo::sync_held_stas(&pool, &a, updated_at(&pool, &a).await, &sync, now)
        .await
        .unwrap();

    assert_eq!(held_rows(&pool, &a).await, vec![("KEEP".to_string(), sta)]);
    assert_eq!(
        held_rows(&pool, &b).await,
        vec![("GONE".to_string(), sta)],
        "the same callsign on another FCA stays"
    );
}

#[sqlx::test]
async fn sync_held_stas_stores_nothing_for_a_read_that_raced_an_edit(pool: PgPool) {
    let id = corridor_fca(&pool, 30).await;
    let now = Utc::now();
    let metered_under = updated_at(&pool, &id).await;
    sqlx::query("update flow.fca set rate = 20 where id = $1")
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
    let store = [("AIR1".to_string(), now.timestamp_millis() + 600_000)];
    let sync = flow_repo::HeldStaSync {
        store: &store,
        drop: &[],
    };

    flow_repo::sync_held_stas(&pool, &id, metered_under, &sync, now)
        .await
        .unwrap();
    assert!(
        held_rows(&pool, &id).await.is_empty(),
        "spaced under the old settings, so not stored"
    );
    // Positive control: the same write under the current version is stored.
    flow_repo::sync_held_stas(&pool, &id, updated_at(&pool, &id).await, &sync, now)
        .await
        .unwrap();
    assert_eq!(held_rows(&pool, &id).await.len(), 1);
}

#[sqlx::test]
async fn a_read_drops_the_held_sta_of_a_flight_gone_from_the_feed(pool: PgPool) {
    let state = state_with(pool.clone(), vec![airborne("AIR1")]).await;
    let id = corridor_fca(&pool, 30).await;
    seed_held(&pool, &id, "GONE", Utc::now().timestamp_millis() + 600_000).await;

    let rows = traffic(&state, &id).await;
    assert_eq!(
        held_rows(&pool, &id).await,
        vec![("AIR1".to_string(), ms(&row(&rows, "AIR1")["cross_time"]))],
        "GONE is no longer crossing, so its row goes; AIR1 is held"
    );
}

#[sqlx::test]
async fn a_one_minute_horizon_does_not_freeze_a_flight_minutes_out(pool: PgPool) {
    let state = state_with(pool.clone(), vec![airborne("AIR1")]).await;
    let id = corridor_fca(&pool, 1).await;

    let rows = traffic(&state, &id).await;
    let air = row(&rows, "AIR1");
    assert!(
        ms(&air["eta"]) - Utc::now().timestamp_millis() > 2 * MIN,
        "the fixture crosses more than a minute out"
    );
    assert_eq!(
        air["sta_frozen"], false,
        "1 min is 60 000 ms, not 1 ms or 60 s per unit"
    );
    assert!(held_rows(&pool, &id).await.is_empty());
}

#[sqlx::test]
async fn a_held_sta_stays_put_when_the_pilot_moves_on(pool: PgPool) {
    // AC2 of #817, end to end: the pilot flies about 13 nm on between reads, which moves the ETA
    // earlier; the STA stored on the first read is served unchanged.
    let state = state_with(pool.clone(), vec![airborne("AIR1")]).await;
    let id = corridor_fca(&pool, 30).await;
    let first = row(&traffic(&state, &id).await, "AIR1").clone();
    {
        let mut feed = state.feed.write().await;
        feed.snapshot = Some(Arc::new(crate::feed::Snapshot::of(VatsimData {
            pilots: vec![Pilot {
                latitude: 40.30,
                longitude: -74.30,
                ..airborne("AIR1")
            }],
            ..Default::default()
        })));
    }

    let second = row(&traffic(&state, &id).await, "AIR1").clone();
    assert!(
        ms(&second["eta"]) < ms(&first["eta"]) - 30_000,
        "positive control: the ETA moved earlier ({} -> {})",
        first["eta"],
        second["eta"]
    );
    assert_eq!(second["cross_time"], first["cross_time"], "the STA holds");
    assert_eq!(second["sta_frozen"], true);
}

/// A session cookie for a user holding `perm` nationally.
async fn national(pool: &PgPool, perm: &str) -> String {
    let user = seed_user(pool).await;
    grant(pool, &user, perm, None).await;
    session_cookie(pool, &user).await
}

/// One request through the real router with an optional JSON body, returning the decoded body.
async fn call(
    state: &AppState,
    method: Method,
    uri: &str,
    cookie: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
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
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn release(state: &AppState, cookie: &str, fca: &str, cs: &str, body: Value) -> Vec<Value> {
    let (status, body) = call(
        state,
        Method::POST,
        &format!("/api/v1/flow/fcas/{fca}/release/{cs}"),
        cookie,
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body.as_array().unwrap().clone()
}

/// AIR1 held at GND1's ETA, so a release that reads the held STA must space GND1 behind it.
async fn held_at_ground_eta(state: &AppState, pool: &PgPool, id: &str) -> i64 {
    let eta_g = ms(&row(&traffic(state, id).await, "GND1")["eta"]);
    sqlx::query("delete from flow.fca_held_sta")
        .execute(pool)
        .await
        .unwrap();
    seed_held(pool, id, "AIR1", eta_g).await;
    eta_g
}

#[sqlx::test]
async fn rdy_spaces_a_departure_behind_a_held_airborne_sta(pool: PgPool) {
    let state = state_with(pool.clone(), vec![airborne("AIR1"), parked("GND1")]).await;
    let id = corridor_fca(&pool, 30).await;
    let held = held_at_ground_eta(&state, &pool, &id).await;
    let cookie = national(&pool, "flow.fca.update").await;

    let rows = release(&state, &cookie, &id, "GND1", json!({})).await;
    let g = row(&rows, "GND1");
    assert_eq!(g["released"], true);
    assert!(
        ms(&g["cross_time"]) >= held + 115_000,
        "the slot is 120 s behind AIR1's held STA, not at GND1's ETA: {g}"
    );
    assert_eq!(ms(&row(&rows, "AIR1")["cross_time"]), held);
}

#[sqlx::test]
async fn clearing_a_release_re_meters_with_the_held_sta(pool: PgPool) {
    let state = state_with(pool.clone(), vec![airborne("AIR1"), parked("GND1")]).await;
    let id = corridor_fca(&pool, 30).await;
    let held = held_at_ground_eta(&state, &pool, &id).await;
    let cookie = national(&pool, "flow.fca.update").await;
    release(&state, &cookie, &id, "GND1", json!({})).await;

    let (status, body) = call(
        &state,
        Method::DELETE,
        &format!("/api/v1/flow/fcas/{id}/release/GND1"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rows = body.as_array().unwrap();
    assert_eq!(
        ms(&row(rows, "AIR1")["cross_time"]),
        held,
        "AIR1 is at its held STA"
    );
    assert!(ms(&row(rows, "GND1")["cross_time"]) >= held + 115_000);
}

#[sqlx::test]
async fn idst_spaces_a_departure_behind_a_held_airborne_sta(pool: PgPool) {
    let state = state_with(pool.clone(), vec![airborne("AIR1"), parked("GND1")]).await;
    let id = corridor_fca(&pool, 30).await;
    let held = held_at_ground_eta(&state, &pool, &id).await;
    let cookie = national(&pool, "flow.fca.read").await;

    let (status, body) = call(
        &state,
        Method::GET,
        "/api/v1/flow/idst?airports=KJFK",
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows = body["unscheduled"].as_array().unwrap();
    let g = row(rows, "GND1");
    assert_eq!(g["fca_id"], id.as_str());
    assert!(
        ms(&g["cross_time"]) >= held + 115_000,
        "IDST reads the held STA too: {g}"
    );
}

#[sqlx::test]
async fn two_departures_set_for_the_same_time_are_both_flagged(pool: PgPool) {
    let state = state_with(pool.clone(), vec![parked("GND1"), parked("GND2")]).await;
    let id = corridor_fca(&pool, 30).await;
    let cookie = national(&pool, "flow.fca.update").await;
    let ready = (Utc::now() + chrono::Duration::minutes(40))
        .format("%H%M")
        .to_string();

    let first = release(&state, &cookie, &id, "GND1", json!({ "ready": ready })).await;
    assert_eq!(
        row(&first, "GND1")["slot_conflict"],
        false,
        "alone it is clear"
    );
    let rows = release(&state, &cookie, &id, "GND2", json!({ "ready": ready })).await;
    assert_eq!(
        row(&rows, "GND1")["cross_time"],
        row(&rows, "GND2")["cross_time"]
    );
    assert_eq!(row(&rows, "GND1")["slot_conflict"], true);
    assert_eq!(row(&rows, "GND2")["slot_conflict"], true);
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

async fn put_fca(state: &AppState, cookie: &str, id: &str, extra: Value) -> StatusCode {
    let mut body = json!({
        "name": "FRZ", "artcc": "ZDC", "points": [[39.5, -75.6], [39.5, -74.0]],
        "mode": "rate", "rate": 20
    });
    body.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    send(
        state,
        Method::PUT,
        &format!("/api/v1/flow/fcas/{id}"),
        cookie,
        Some(body),
    )
    .await
}

#[sqlx::test]
async fn an_fca_edit_clears_its_held_stas_and_keeps_an_omitted_horizon(pool: PgPool) {
    let state = state_with(pool.clone(), vec![]).await;
    let id = corridor_fca(&pool, 45).await;
    let other = corridor_fca(&pool, 30).await;
    seed_held(&pool, &id, "AIR1", Utc::now().timestamp_millis()).await;
    seed_held(&pool, &other, "AIR2", Utc::now().timestamp_millis()).await;
    let cookie = national(&pool, "flow.fca.update").await;

    assert_eq!(
        put_fca(&state, &cookie, &id, json!({})).await,
        StatusCode::OK
    );
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
    assert_eq!(
        fca.freeze_min, 45,
        "a request without the field keeps the stored horizon"
    );
    assert_eq!(fca.rate, 20, "positive control: the edit applied");

    assert_eq!(
        put_fca(&state, &cookie, &id, json!({ "freeze_min": 12 })).await,
        StatusCode::OK
    );
    let fca = flow_repo::get_fca(&pool, &id).await.unwrap().unwrap();
    assert_eq!(fca.freeze_min, 12);
}

#[sqlx::test]
async fn a_horizon_outside_0_to_120_min_is_refused(pool: PgPool) {
    let state = state_with(pool.clone(), vec![]).await;
    let id = corridor_fca(&pool, 30).await;
    let cookie = national(&pool, "flow.fca.update").await;
    for bad in [-1, 121, 500] {
        assert_eq!(
            put_fca(&state, &cookie, &id, json!({ "freeze_min": bad })).await,
            StatusCode::BAD_REQUEST,
            "{bad} min"
        );
    }
    let create = json!({
        "name": "NEW", "artcc": "ZDC", "points": [[39.5, -75.6], [39.5, -74.0]],
        "mode": "rate", "rate": 20, "freeze_min": 121
    });
    assert_eq!(
        send(
            &state,
            Method::POST,
            "/api/v1/flow/fcas",
            &cookie,
            Some(create)
        )
        .await,
        StatusCode::BAD_REQUEST,
        "a create is bounded too"
    );
    for good in [0, 120] {
        assert_eq!(
            put_fca(&state, &cookie, &id, json!({ "freeze_min": good })).await,
            StatusCode::OK,
            "{good} min is in range"
        );
    }
    let fca = flow_repo::get_fca(&pool, &id).await.unwrap().unwrap();
    assert_eq!(fca.freeze_min, 120);
}

#[sqlx::test]
async fn deleting_an_fca_clears_its_held_stas(pool: PgPool) {
    let state = state_with(pool.clone(), vec![]).await;
    let id = corridor_fca(&pool, 30).await;
    let other = corridor_fca(&pool, 30).await;
    seed_held(&pool, &id, "AIR1", Utc::now().timestamp_millis()).await;
    seed_held(&pool, &other, "AIR2", Utc::now().timestamp_millis()).await;
    let cookie = national(&pool, "flow.fca.delete").await;

    let status = send(
        &state,
        Method::DELETE,
        &format!("/api/v1/flow/fcas/{id}"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(held_rows(&pool, &id).await.is_empty());
    assert_eq!(
        held_rows(&pool, &other).await.len(),
        1,
        "another FCA's rows stay"
    );
}

#[sqlx::test]
async fn a_signed_in_read_of_a_deleted_fca_stores_no_held_stas(pool: PgPool) {
    let state = state_with(pool.clone(), vec![airborne("AIR1")]).await;
    let id = corridor_fca(&pool, 30).await;
    let cookie = national(&pool, "flow.fca.delete").await;
    let uri = format!("/api/v1/flow/fcas/{id}");
    assert_eq!(
        send(&state, Method::DELETE, &uri, &cookie, None).await,
        StatusCode::NO_CONTENT
    );

    let (status, body) = call(
        &state,
        Method::GET,
        &format!("{uri}/traffic"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        row(body.as_array().unwrap(), "AIR1")["sta_frozen"],
        true,
        "positive control: the read metered AIR1 inside the horizon"
    );
    assert!(
        held_rows(&pool, &id).await.is_empty(),
        "nothing is stored for a deleted FCA"
    );
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
