//! The FCA freeze horizon's stored STAs (#817): what the held-STA pass (`sync_fca_holds_once`)
//! writes, what every read serves without writing, and what prunes or clears them.
//!
//! The router tests use the corridor feed from `flow_prefile_candidate_tests`: an FCA between WHITE
//! and SIE on the KJFK->KDCA route, and an aircraft climbing out of KJFK that crosses it airborne
//! inside the default 30 min horizon.
//!
//! The pass takes its clock; the routes read `Utc::now()`. The feed snapshot here never moves, so a
//! flight's ETA (now + a fixed time to the line) grows with the clock between calls. Tests that
//! assert a read serves the pass's STA exactly run the pass 60 s ahead of the wall clock, so the
//! read that follows sees the flight on time and the STA unchanged, not late by a few ms.
//
// Coverage matrix (VATUSA/OIS#817, rework round: the pass is the only writer)
//
// Requirement                                   | Test                                                              | Status
// ----------------------------------------------|-------------------------------------------------------------------|-------
// pass stores airborne, not ground              | the_pass_holds_an_airborne_flights_sta_and_not_a_ground_one        | DONE
// read serves a held STA while ETA differs      | a_held_sta_is_served_on_the_next_read_while_the_eta_differs        | DONE
// late held flight crosses at ETA               | a_late_held_flight_crosses_at_its_eta_and_shows_lateness           | DONE
// freeze_min 0 stores nothing                   | freezing_off_stores_nothing                                        | DONE
// stale row ignored by read, replaced by pass   | a_stale_held_row_is_ignored_by_the_read_then_replaced_by_the_pass  | DONE
// retention constant pinned                     | the_retention_window_sits_between_the_fixtures                     | DONE
// sync_held_stas repo contract                  | sync_held_stas_* (3)                                               | DONE
// pass drops a flight gone from the feed        | the_pass_drops_the_held_sta_of_a_flight_gone_from_the_feed         | DONE
// 1 min horizon is 60 000 ms                    | a_one_minute_horizon_does_not_freeze_a_flight_minutes_out          | DONE
// AC2: STA holds as the pilot moves on          | a_held_sta_stays_put_when_the_pilot_moves_on                       | DONE
// a read never prunes or rewrites a hold        | a_read_never_prunes_or_rewrites_the_held_stas                      | DONE
// GET /traffic anon + signed in writes nothing  | a_traffic_read_signed_out_or_in_writes_no_held_row                 | TODO
// pass: store live, prune 4 unmetered kinds     | the_pass_stores_for_live_fcas_and_prunes_every_unmetered_fca       | TODO
// pass: one FCA failing does not stop the rest  | one_fcas_failed_sync_does_not_stop_the_pass                        | TODO
// hysteresis: held kept to +2 min, then dropped | a_held_flight_stays_frozen_up_to_two_minutes_past_the_horizon     | TODO
// reorder: real change clears, re-held by ETA   | a_real_reorder_clears_the_holds_and_the_pass_re_holds_in_the_new_order | TODO
// reorder: no-op save keeps; swap / auto clears | a_manual_order_save_that_changes_nothing_keeps_the_holds          | TODO
// PUT: name/color/artcc/enabled keep holds      | an_edit_to_name_color_artcc_or_enabled_keeps_the_held_stas        | TODO
// PUT: each metering field alone clears holds   | an_edit_to_any_one_metering_field_clears_the_held_stas            | TODO
// released flight's hold dropped (RDY and SET)  | the_pass_drops_the_hold_of_a_flight_released_by_rdy_or_set        | TODO
// RDY/clear/IDST read the hold, exact, no write | rdy_*, clearing_a_release_*, idst_* (exact values)                 | DONE
// flight advisory serves the held STA           | the_flight_advisory_serves_the_held_sta                            | TODO
// create with freeze_min 45 round-trips         | a_create_with_a_45_min_horizon_round_trips                         | TODO
// edit clears / omitted horizon kept            | an_fca_edit_clears_its_held_stas_and_keeps_an_omitted_horizon      | DONE
// horizon bounds 0..=120                        | a_horizon_outside_0_to_120_min_is_refused                          | DONE
// delete clears; deleted FCA never re-held      | deleting_an_fca_clears_its_held_stas, a_deleted_fca_gets_no_held_stas_from_a_read_or_the_pass | DONE
// default horizon on create (both paths)        | a_new_fca_gets_the_default_horizon                                 | DONE

use std::collections::HashMap;
use std::sync::Arc;

use axum::http::{Method, StatusCode};
use chrono::Utc;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::feed::airports::Airport;
use crate::feed::vatsim::{FlightPlan, Pilot, VatsimData};
use crate::handlers::flow::sync_fca_holds_once;
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
async fn the_pass_holds_an_airborne_flights_sta_and_not_a_ground_one(pool: PgPool) {
    let state = state_with(pool.clone(), vec![airborne("AIR1"), parked("GND1")]).await;
    let id = corridor_fca(&pool, 30).await;

    // 60 s ahead of the wall clock: see the module doc.
    sync_fca_holds_once(&state, Utc::now() + chrono::Duration::seconds(60))
        .await
        .unwrap();
    let rows = traffic(&state, &id).await;
    let air = row(&rows, "AIR1");
    assert_eq!(air["sta_frozen"], true, "minutes out is inside 30 min");
    assert_eq!(row(&rows, "GND1")["sta_frozen"], false);
    assert_eq!(
        held_rows(&pool, &id).await,
        vec![("AIR1".to_string(), ms(&air["cross_time"]))],
        "the pass stores the airborne flight's STA, and only that one; the read serves it"
    );
}

#[sqlx::test]
async fn a_held_sta_is_served_on_the_next_read_while_the_eta_differs(pool: PgPool) {
    let state = state_with(pool.clone(), vec![airborne("AIR1")]).await;
    let id = corridor_fca(&pool, 30).await;
    let eta = ms(&row(&traffic(&state, &id).await, "AIR1")["eta"]);
    assert!(
        held_rows(&pool, &id).await.is_empty(),
        "the read stored nothing"
    );
    // A stored STA 5 min after the ETA, as if the ETA had moved since the pass held it.
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
    assert!(
        held_rows(&pool, &id).await.is_empty(),
        "the read stored nothing"
    );
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

    sync_fca_holds_once(&state, Utc::now()).await.unwrap();
    assert_eq!(
        row(&traffic(&state, &off).await, "AIR1")["sta_frozen"],
        false
    );
    assert!(held_rows(&pool, &off).await.is_empty());
    // Positive control: the same pass held the same flight on an FCA with a horizon.
    assert_eq!(held_rows(&pool, &on).await.len(), 1);
}

#[sqlx::test]
async fn a_stale_held_row_is_ignored_by_the_read_then_replaced_by_the_pass(pool: PgPool) {
    let state = state_with(pool.clone(), vec![airborne("AIR1")]).await;
    let id = corridor_fca(&pool, 30).await;
    let stale = Utc::now().timestamp_millis() - 2 * flow_repo::HELD_STA_RETAIN_MS;
    seed_held(&pool, &id, "AIR1", stale).await;

    let rows = traffic(&state, &id).await;
    assert_eq!(
        row(&rows, "AIR1")["late_sec"],
        0,
        "a stale row is not a held STA"
    );
    assert_eq!(
        held_rows(&pool, &id).await,
        vec![("AIR1".to_string(), stale)],
        "the read leaves the stale row for the pass"
    );

    // 60 s ahead of the wall clock: see the module doc.
    sync_fca_holds_once(&state, Utc::now() + chrono::Duration::seconds(60))
        .await
        .unwrap();
    let rows = traffic(&state, &id).await;
    assert_eq!(
        held_rows(&pool, &id).await,
        vec![("AIR1".to_string(), ms(&row(&rows, "AIR1")["cross_time"]))],
        "the pass prunes the stale row and holds the current STA"
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
async fn the_pass_drops_the_held_sta_of_a_flight_gone_from_the_feed(pool: PgPool) {
    let state = state_with(pool.clone(), vec![airborne("AIR1")]).await;
    let id = corridor_fca(&pool, 30).await;
    let gone = Utc::now().timestamp_millis() + 600_000;
    seed_held(&pool, &id, "GONE", gone).await;

    traffic(&state, &id).await;
    assert_eq!(
        held_rows(&pool, &id).await,
        vec![("GONE".to_string(), gone)],
        "the read drops nothing and stores nothing"
    );

    // 60 s ahead of the wall clock: see the module doc.
    sync_fca_holds_once(&state, Utc::now() + chrono::Duration::seconds(60))
        .await
        .unwrap();
    let rows = traffic(&state, &id).await;
    assert_eq!(
        held_rows(&pool, &id).await,
        vec![("AIR1".to_string(), ms(&row(&rows, "AIR1")["cross_time"]))],
        "GONE is no longer crossing, so the pass drops its row; AIR1 is held"
    );
}

#[sqlx::test]
async fn a_one_minute_horizon_does_not_freeze_a_flight_minutes_out(pool: PgPool) {
    let state = state_with(pool.clone(), vec![airborne("AIR1")]).await;
    let id = corridor_fca(&pool, 1).await;

    sync_fca_holds_once(&state, Utc::now()).await.unwrap();
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
    // AC2 of #817, end to end: the pilot flies about 13 nm on between passes, which moves the ETA
    // earlier; the STA the first pass stored is kept by the second and served unchanged.
    let state = state_with(pool.clone(), vec![airborne("AIR1")]).await;
    let id = corridor_fca(&pool, 30).await;
    // 60 s ahead of the wall clock: see the module doc.
    sync_fca_holds_once(&state, Utc::now() + chrono::Duration::seconds(60))
        .await
        .unwrap();
    let first = row(&traffic(&state, &id).await, "AIR1").clone();
    let stored = held_rows(&pool, &id).await;
    assert_eq!(stored, vec![("AIR1".to_string(), ms(&first["cross_time"]))]);
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

    sync_fca_holds_once(&state, Utc::now() + chrono::Duration::seconds(60))
        .await
        .unwrap();
    assert_eq!(
        held_rows(&pool, &id).await,
        stored,
        "the second pass keeps the first STA"
    );
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

/// AIR1 held at GND1's ETA, so a release that reads the held STA must space GND1 behind it. GND1's
/// ETA drifts by the ms between this read and the next request, far inside the 120 s gap, so the
/// slot behind AIR1 is exact.
async fn held_at_ground_eta(state: &AppState, pool: &PgPool, id: &str) -> i64 {
    let eta_g = ms(&row(&traffic(state, id).await, "GND1")["eta"]);
    assert!(
        held_rows(pool, id).await.is_empty(),
        "the read stored nothing"
    );
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
    assert_eq!(
        ms(&g["cross_time"]),
        held + 135_000,
        "rate 30's 120 s gap plus RDY's 15 s margin behind AIR1's held STA, not GND1's ETA: {g}"
    );
    assert_eq!(ms(&row(&rows, "AIR1")["cross_time"]), held);
    assert_eq!(
        held_rows(&pool, &id).await,
        vec![("AIR1".to_string(), held)],
        "RDY reads the hold and writes none"
    );
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
    assert_eq!(
        ms(&row(rows, "GND1")["cross_time"]),
        held + 120_000,
        "unreleased again, GND1 is one 120 s gap behind the held STA (no release margin)"
    );
    assert_eq!(
        held_rows(&pool, &id).await,
        vec![("AIR1".to_string(), held)],
        "clearing a release reads the hold and writes none"
    );
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
    assert_eq!(
        ms(&g["cross_time"]),
        held + 120_000,
        "IDST reads the held STA too, one 120 s gap behind it: {g}"
    );
    assert_eq!(
        held_rows(&pool, &id).await,
        vec![("AIR1".to_string(), held)],
        "IDST reads the hold and writes none"
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
async fn a_read_never_prunes_or_rewrites_the_held_stas(pool: PgPool) {
    // The pass holds AIR1; a stale row sits beside it. A read serves AIR1's hold and leaves both
    // rows exactly as they were: pruning is the pass's job.
    let state = state_with(pool.clone(), vec![airborne("AIR1")]).await;
    let id = corridor_fca(&pool, 30).await;
    // 60 s ahead of the wall clock: see the module doc.
    sync_fca_holds_once(&state, Utc::now() + chrono::Duration::seconds(60))
        .await
        .unwrap();
    let air = held_rows(&pool, &id).await;
    assert_eq!(air.len(), 1, "positive control: the pass held AIR1");
    let stale = Utc::now().timestamp_millis() - flow_repo::HELD_STA_RETAIN_MS - 60_000;
    seed_held(&pool, &id, "GONE", stale).await;

    let rows = traffic(&state, &id).await;
    assert_eq!(
        row(&rows, "AIR1")["sta_frozen"],
        true,
        "positive control: AIR1 is held"
    );
    assert_eq!(ms(&row(&rows, "AIR1")["cross_time"]), air[0].1);
    assert_eq!(
        held_rows(&pool, &id).await,
        vec![air[0].clone(), ("GONE".to_string(), stale)],
        "the read pruned nothing and AIR1's first STA is unchanged"
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
async fn a_deleted_fca_gets_no_held_stas_from_a_read_or_the_pass(pool: PgPool) {
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
        "a signed-in read of a deleted FCA stores nothing"
    );
    let live = corridor_fca(&pool, 30).await;
    sync_fca_holds_once(&state, Utc::now()).await.unwrap();
    assert!(
        held_rows(&pool, &id).await.is_empty(),
        "the pass does not meter a deleted FCA"
    );
    assert_eq!(
        held_rows(&pool, &live).await.len(),
        1,
        "positive control: the same pass held AIR1 on a live FCA on the same line"
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

// --- rework round: the pass is the only writer (skeletons) ---

// Requirement 2: GET /traffic only reads holds, signed out and signed in.
#[sqlx::test]
#[ignore = "todo: a traffic read, anonymous or signed in, writes no held row"]
async fn a_traffic_read_signed_out_or_in_writes_no_held_row(_pool: PgPool) {
    // Setup: state_with([airborne("AIR1")]); id = corridor_fca(30).
    // Action 1: traffic(&state, &id) (anonymous).
    // Assert: AIR1 sta_frozen == true (positive control: the read metered it inside the
    //         horizon), and held_rows(id) is empty.
    // Action 2: cookie = national("flow.fca.read"); send_json(GET /api/v1/flow/fcas/{id}/traffic).
    // Assert: 200, AIR1 sta_frozen == true, held_rows(id) still empty.
    // Positive control: sync_fca_holds_once(&state, Utc::now() + 60 s) then
    //         held_rows(id) == [("AIR1", ms(cross_time of a read that follows))].
    todo!()
}

// Requirement 3: one tick stores for every metered FCA and prunes the rows of every FCA it does not
// meter, with one surviving neighbor per predicate of `prune_unmetered_held_stas`'s WHERE
// (`f.id = h.fca_id`, `f.enabled`, `f.deleted_at is null`, `event_id is null or status = 'published'`).
#[sqlx::test]
#[ignore = "todo: the pass stores for live FCAs and prunes disabled, deleted, planned and archived ones in one tick"]
async fn the_pass_stores_for_live_fcas_and_prunes_every_unmetered_fca(_pool: PgPool) {
    // Setup: state_with([airborne("AIR1")]); insert events.event id 817.
    //   Six corridor FCAs inserted inline (rate 30, freeze_min 30, explicit enabled / deleted_at /
    //   event_id / event_status on every row):
    //     LIVE  enabled, not deleted, no event       -> nothing seeded; the pass stores AIR1
    //     PUB   enabled, event 817, 'published'      -> seed AIR1 at now_ms + 600_000; kept unchanged
    //     OFF   enabled = false                      -> seed AIR1; pruned  (neighbor of f.enabled: LIVE)
    //     DEL   enabled, deleted_at = now()          -> seed AIR1; pruned  (neighbor: LIVE)
    //     PLAN  enabled, event 817, 'planned'        -> seed AIR1; pruned  (neighbor of the event
    //                                                   predicate: PUB; of `event_id is null`: LIVE)
    //     ARCH  enabled, event 817, 'archived'       -> seed AIR1; pruned
    //   Same callsign everywhere, so only the FCA's state decides each row.
    //   Assert before the pass: 5 rows seeded.
    // Action: sync_fca_holds_once(&state, Utc::now() + 60 s) -> Ok(()), one call.
    // Assert:
    //   held_rows(LIVE) == [("AIR1", ms(cross_time of a LIVE traffic read))]  (stored this tick)
    //   held_rows(PUB)  == [("AIR1", seeded)]                                  (metered, untouched)
    //   held_rows(OFF), (DEL), (PLAN), (ARCH) each empty, the message naming the FCA.
    // Mutations that must go red: drop `f.enabled` (OFF survives), drop `deleted_at is null` (DEL
    // survives), drop the event predicate (PLAN/ARCH survive), drop `event_id is null or` (LIVE
    // pruned), drop `f.id = h.fca_id` (nothing pruned).
    todo!()
}

// Requirement 4: every FCA is attempted; a failure on one is reported and does not stop the others
// or the prune. The signature returns Err when any FCA failed, so this asserts Err, not Ok.
#[sqlx::test]
#[ignore = "todo: one FCA's failed sync is reported and the other FCAs and the prune still run"]
async fn one_fcas_failed_sync_does_not_stop_the_pass(_pool: PgPool) {
    // Setup: state_with([airborne("AIR1")]). Three corridor FCAs inserted inline with explicit
    //   names so list_fcas (order by name) meters the failing one FIRST:
    //     "AAA" enabled (bad), "ZZZ" enabled (good), "OFF" enabled = false with AIR1 seeded.
    //   Inject the failure at the write it guards, inside this test's own database (DDL is
    //   contained by #[sqlx::test]):
    //     create function public.fail_held_sta() returns trigger language plpgsql
    //       as $$ begin raise exception 'injected'; end $$;
    //     create trigger fail_held before insert on flow.fca_held_sta for each row
    //       when (new.fca_id = '<AAA id>') execute function public.fail_held_sta();
    //   (the id is formatted in: a value this test inserted, not input). Separate sqlx::query calls.
    // Action: let r = sync_fca_holds_once(&state, Utc::now()).await;
    // Assert:
    //   r is Err and its message contains AAA's id (the failed write was reached: the trigger only
    //     fires on the insert of AAA's AIR1 row);
    //   held_rows(AAA) empty (the transaction rolled back);
    //   held_rows(ZZZ) == one AIR1 row (metered after the failure);
    //   held_rows(OFF) empty (the prune ran after the failure).
    todo!()
}

// Hysteresis at the pass (the unit tests live in feed/fca.rs). The pass cannot be given an exact
// ETA offset: ETA = now + a time to the line fixed by the feed geometry, so the brief's "31 min out
// keeps / 32 min + 1 ms drops" is expressed as horizons chosen around the measured offset instead.
#[sqlx::test]
#[ignore = "todo: a held flight stays frozen up to 2 min past the horizon and is dropped beyond it"]
async fn a_held_flight_stays_frozen_up_to_two_minutes_past_the_horizon(_pool: PgPool) {
    // Setup: state_with([airborne("AIR1")]); now = Utc::now(); now_ms = now.timestamp_millis().
    //   Probe FCA inserted inline: rate 0 (no spacing), freeze_min 120. One pass at `now` stores
    //   AIR1 at its unmetered ETA: eta = held_rows(probe)[0].1; out = eta - now_ms.
    //   Assert out > 4 * MIN (so the "beyond" horizon is >= 1 min, not 0 = freezing off).
    //   inside = (out - 120_000 + MIN - 1) / MIN   // ceil: smallest whole-minute horizon whose
    //                                               // 2 min margin still covers AIR1
    //   beyond = inside - 1
    //   Assert inside * MIN < out <= inside * MIN + 120_000 and out > beyond * MIN + 120_000, and
    //   inside <= 120 (literal 120_000, not FREEZE_HYSTERESIS_MS, so a changed margin goes red).
    //   kept    = corridor_fca(inside), seed AIR1 at eta
    //   fresh   = corridor_fca(inside), nothing seeded
    //   dropped = corridor_fca(beyond), seed AIR1 at eta
    // Action: sync_fca_holds_once(&state, now) (same `now`, so the same ETA).
    // Assert:
    //   held_rows(kept)    == [("AIR1", eta)]   past the horizon, inside the margin: kept
    //   held_rows(fresh)   is empty             no stored STA: the plain horizon applies
    //   held_rows(dropped) is empty             past horizon + margin: dropped
    // Mutations: drop the hysteresis (kept goes red); make a held flight frozen at any distance
    // (dropped goes red); widen the margin to new flights too (fresh goes red).
    todo!()
}

// Requirement 5: a real reorder clears the old order's holds; the pass re-holds from the ETAs.
#[sqlx::test]
#[ignore = "todo: a real reorder clears the holds and the pass re-holds A at its ETA and B one gap behind"]
async fn a_real_reorder_clears_the_holds_and_the_pass_re_holds_in_the_new_order(_pool: PgPool) {
    // The reviewer's case: rate 30 is a 120 s gap. B crosses first; B is held a minute after its
    // ETA and A one gap behind B. Dragging A first must re-meter from the ETAs, not anchor A at its
    // old STA and push B a gap behind that (A held_a / B held_a + 120 s).
    // Setup: state_with([airborne("AIRA"), Pilot { latitude: 40.38, longitude: -74.20,
    //   ..airborne("AIRB") }]); id = corridor_fca(30); probe = inline rate-0 FCA on the same line.
    //   now = Utc::now() + 60 s (module doc), used for EVERY pass in this test.
    //   Pass 1 at now. etas = held_rows(probe) == [("AIRA", eta_a), ("AIRB", eta_b)].
    //   Assert eta_b < eta_a < eta_b + 180_000 (B first; A on time at the hold below; the buggy
    //   and the fixed answers differ).
    //   Replace id's holds: delete where fca_id = id; held_b = eta_b + 60_000; held_a = eta_b + 180_000.
    //   Positive control: traffic(id): AIRB cross_time == held_b, AIRA == held_a.
    // Action: cookie = national("flow.fca.update");
    //   send(PUT /api/v1/flow/fcas/{id}/order, {"order": ["AIRA", "AIRB"]}) == 204.
    // Assert: held_rows(id) empty ("the reorder clears the old order's holds");
    //   held_rows(probe) == etas ("and no other FCA's").
    // Action: pass 2 at now.
    // Assert: held_rows(id) == [("AIRA", eta_a), ("AIRB", eta_a + 120_000)] with a message naming
    //   the buggy pair (held_a, held_a + 120_000); traffic(id): AIRA cross_time == eta_a, seq 1;
    //   AIRB cross_time == eta_a + 120_000, seq 2.
    todo!()
}

// Requirement 5: only a change of manual_order or manual_seq clears.
#[sqlx::test]
#[ignore = "todo: a manual order save that changes nothing keeps the holds; a same-size swap and back-to-auto clear"]
async fn a_manual_order_save_that_changes_nothing_keeps_the_holds(_pool: PgPool) {
    // Setup: the two-flight feed of the reorder test; manual = corridor_fca(30) and
    //   auto = corridor_fca(30) (stored manual_order '{}', manual_seq false: migration 0023's
    //   defaults, asserted by reading the row). now = Utc::now() + 60 s for every pass.
    // Step 1, auto no-op: pass; before = held_rows(auto) (len 2);
    //   PUT /fcas/{auto}/order {"order": []} == 204; held_rows(auto) == before.
    // Step 2, manual no-op: PUT /fcas/{manual}/order ["AIRA","AIRB"] == 204 (clears); pass;
    //   before = held_rows(manual) (len 2); PUT the same order again == 204;
    //   held_rows(manual) == before ("an identical save keeps the holds").
    // Step 3, same-size swap (identity, not length): PUT ["AIRB","AIRA"] == 204;
    //   held_rows(manual) empty. Positive control: held_rows(auto) unchanged.
    // Step 4, manual_seq alone: pass (re-holds); PUT {"order": []} == 204 (manual -> auto);
    //   held_rows(manual) empty.
    todo!()
}

// Requirement 6: name, color, artcc and enabled are not metering fields.
#[sqlx::test]
#[ignore = "todo: an edit to name, color, artcc or enabled keeps the held STAs"]
async fn an_edit_to_name_color_artcc_or_enabled_keeps_the_held_stas(_pool: PgPool) {
    // Setup: state_with([]); id = corridor_fca(30); cookie = national("flow.fca.update").
    //   base = every UpsertFcaRequest field explicit: {"name": "FRZ", "color": "#efc14d",
    //   "artcc": "ZDC", "points": [[39.5, -75.6], [39.5, -74.0]], "dests": [], "origins": [],
    //   "fixes": [], "scope": [], "min_fl": null, "max_fl": null, "dir": "any", "mode": "rate",
    //   "rate": 30, "mit": 15, "freeze_min": 30, "enabled": true}.
    //   PUT base == 200; sta = Utc::now() ms + 600_000; seed_held(id, "AIR1", sta).
    // Loop (field, value) over ("name", "FRZ RENAMED"), ("color", "#ffffff"), ("artcc", "ZNY"),
    //   ("enabled", false); each from base, changed alone:
    //   PUT changed == 200, "{field}";
    //   serde_json::to_value(get_fca(id))[field] == value, "{field} was stored" (positive control);
    //   held_rows(id) == [("AIR1", sta)], "changing {field} alone keeps the held STAs";
    //   PUT base == 200 (back; also not a metering change).
    // Positive control after the loop: PUT base with "rate": 20 == 200; held_rows(id) empty.
    todo!()
}

// Requirement 6: every metering field, changed alone, clears.
#[sqlx::test]
#[ignore = "todo: an edit to any one metering field clears the held STAs"]
async fn an_edit_to_any_one_metering_field_clears_the_held_stas(_pool: PgPool) {
    // Setup: as the keep test (same explicit base, national flow.fca.update cookie).
    // Loop (field, value) over:
    //   ("points", [[39.4, -75.6], [39.4, -74.0]]), ("dests", ["KDCA"]), ("origins", ["KJFK"]),
    //   ("fixes", ["SIE"]), ("scope", ["ZDC_01"]), ("min_fl", 100), ("max_fl", 450),
    //   ("dir", "S"), ("mode", "mit"), ("rate", 20), ("mit", 20), ("freeze_min", 45)
    // Each iteration:
    //   PUT base == 200 ("{field}: reset to base"; clears the previous iteration's change);
    //   seed_held(id, "AIR1", sta);
    //   PUT base == 200 again; held_rows(id) == [("AIR1", sta)], "{field}: an unchanged save keeps";
    //   changed = base with changed[field] = value; PUT changed == 200, "{field}";
    //   serde_json::to_value(get_fca(id))[field] == value, "{field} was stored";
    //   held_rows(id) empty, "changing {field} alone clears the held STAs".
    // Mutation: remove any one column from update_fca's distinct-from tuple; only its entry fails.
    todo!()
}

// Requirement 7: a release ends the hold.
#[sqlx::test]
#[ignore = "todo: the pass drops the hold of a flight released by RDY or SET"]
async fn the_pass_drops_the_hold_of_a_flight_released_by_rdy_or_set(_pool: PgPool) {
    // Setup: state_with([airborne("AIR1"), Pilot { 40.38, -74.20, ..airborne("AIR2") },
    //   Pilot { 40.30, -74.30, ..airborne("AIR3") }]); id = corridor_fca(30).
    //   Pass at Utc::now() + 60 s; before = held_rows(id); assert the three callsigns are held.
    // Action: cookie = national("flow.fca.update");
    //   release(AIR1, {}) (RDY); release(AIR2, {"ready": HHMM of now + 40 min}) (SET).
    // Assert: held_rows(id) == before ("a release writes no hold"; the pass drops them).
    // Action: pass at Utc::now() + 60 s.
    // Assert: held_rows(id) == [before's AIR3 row] exactly (the neighbor keeps its first STA);
    //   traffic(id): AIR1 and AIR2 released == true (positive control: the releases stand).
    todo!()
}

// Requirement 8: the flight advisory reads the hold.
#[sqlx::test]
#[ignore = "todo: the flight advisory's FCA crossing is the held STA"]
async fn the_flight_advisory_serves_the_held_sta(_pool: PgPool) {
    // Setup: state_with([airborne("AIR1")]); id = corridor_fca(30);
    //   eta = ms(traffic(id) AIR1 "eta"); seed_held(id, "AIR1", eta + 300_000).
    // Action: send_json(GET /api/v1/public/flight/AIR1, "") (public route, router.rs).
    // Assert: 200; body["found"] == true; body["fcas"] has one entry with fca_id == id and
    //   ms(cross_time) == eta + 300_000 ("the held STA, not the ETA");
    //   held_rows(id) == [("AIR1", eta + 300_000)] ("the advisory only reads").
    todo!()
}

// Requirement 8: a non-default horizon survives a create.
#[sqlx::test]
#[ignore = "todo: a create with freeze_min 45 round-trips"]
async fn a_create_with_a_45_min_horizon_round_trips(_pool: PgPool) {
    // Setup: state_with([]); cookie = national("flow.fca.update").
    // Action: call(POST /api/v1/flow/fcas, {"name": "NEW", "artcc": "ZDC",
    //   "points": [[39.5, -75.6], [39.5, -74.0]], "mode": "rate", "rate": 20, "freeze_min": 45}).
    // Assert: 200; body["freeze_min"] == 45 (not the default 30);
    //   get_fca(body["id"]).freeze_min == 45.
    todo!()
}
