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
// Coverage matrix (VATUSA/OIS#817: the pass is the only thing that stores holds)
//
// Requirement                                   | Test                                                              | Status
// ----------------------------------------------|-------------------------------------------------------------------|-------
// pass stores airborne, not ground              | the_pass_holds_an_airborne_flights_sta_and_not_a_ground_one        | DONE
// read serves a held STA while ETA differs      | a_held_sta_is_served_on_the_next_read_while_the_eta_differs        | DONE
// late held flight crosses at ETA               | a_late_held_flight_crosses_at_its_eta_and_shows_lateness           | DONE
// late held flight in a gap: flagged, no move  | a_late_held_flight_inside_a_held_neighbors_gap_is_flagged_and_moves_no_held_neighbor | DONE
// freeze_min 0 stores nothing                   | freezing_off_stores_nothing                                        | DONE
// stale row ignored by read, replaced by pass   | a_stale_held_row_is_ignored_by_the_read_then_replaced_by_the_pass  | DONE
// retention constant pinned                     | the_retention_window_sits_between_the_fixtures                     | DONE
// sync_held_stas repo contract                  | sync_held_stas_* (3)                                               | DONE
// pass drops a flight gone from the feed        | the_pass_drops_the_held_sta_of_a_flight_gone_from_the_feed         | DONE
// pass waits for feed, airports, winds, profiles| the_pass_keeps_every_hold_until_its_inputs_are_loaded             | DONE
// a failed loader fails the pass, changes none | the_pass_fails_while_a_failed_loader_has_left_its_input_empty      | DONE
// a release in another's gap is flagged       | the_traffic_route_flags_a_release_left_inside_the_gap              | DONE
// 1 min horizon is 60 000 ms                    | a_one_minute_horizon_does_not_freeze_a_flight_minutes_out          | DONE
// AC2: STA holds as the pilot moves on          | a_held_sta_stays_put_when_the_pilot_moves_on                       | DONE
// a read never prunes or rewrites a hold        | a_read_never_prunes_or_rewrites_the_held_stas                      | DONE
// GET /traffic anon + signed in writes nothing  | a_traffic_read_signed_out_or_in_writes_no_held_row                 | DONE
// pass: store live, prune 4 unmetered kinds     | the_pass_stores_for_live_fcas_and_prunes_every_unmetered_fca       | DONE
// pass: one FCA failing does not stop the rest  | one_fcas_failed_sync_does_not_stop_the_pass                        | DONE
// pass: the error names the failed step         | a_failed_release_read_is_reported_as_that_step                     | DONE
// late held: slot behind a held gap kept       | a_late_held_flight_behind_a_held_neighbors_gap_crosses_at_its_slot | DONE
// late held: slot reserved over two passes      | a_flight_held_beside_a_late_held_flight_keeps_its_sta_on_the_next_pass | DONE
// hysteresis: held kept to +2 min, then dropped | a_held_flight_stays_frozen_up_to_two_minutes_past_the_horizon     | DONE
// reorder: real change clears, re-held by ETA   | a_real_reorder_clears_the_holds_and_the_pass_re_holds_in_the_new_order | DONE
// reorder: no-op save keeps; swap / auto clears | a_manual_order_save_that_changes_nothing_keeps_the_holds          | DONE
// PUT: name/color/artcc/enabled keep holds      | an_edit_to_name_color_artcc_or_enabled_keeps_the_held_stas        | DONE
// PUT: each metering field alone clears holds   | an_edit_to_any_one_metering_field_clears_the_held_stas            | DONE
// released flight's hold dropped (RDY and SET)  | the_pass_drops_the_hold_of_a_flight_released_by_rdy_or_set        | DONE
// RDY/clear/IDST read the hold, exact, no write | rdy_*, clearing_a_release_*, idst_* (exact values)                 | DONE
// flight advisory serves the held STA           | the_flight_advisory_serves_the_held_sta                            | DONE
// create with freeze_min 45 round-trips         | a_create_with_a_45_min_horizon_round_trips                         | DONE
// edit clears / omitted horizon kept            | an_fca_edit_clears_its_held_stas_and_keeps_an_omitted_horizon      | DONE
// horizon bounds 0..=120                        | a_horizon_outside_0_to_120_min_is_refused                          | DONE
// delete clears; deleted FCA never re-held      | deleting_an_fca_clears_its_held_stas, a_deleted_fca_gets_no_held_stas_from_a_read_or_the_pass | DONE
// default horizon on create (both paths)        | a_new_fca_gets_the_default_horizon                                 | DONE

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

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
    // The hold pass waits until winds and profiles have loaded once (`hold_inputs_ready`).
    state
        .winds_refreshed
        .store(Utc::now().timestamp_millis(), Ordering::Relaxed);
    state
        .aircraft_profiles_loaded
        .store(true, Ordering::Relaxed);
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
async fn a_late_held_flight_inside_a_held_neighbors_gap_is_flagged_and_moves_no_held_neighbor(
    pool: PgPool,
) {
    // AIRA was held 60 s before its ETA, so it runs 60 s late. AIRB, which crosses about 50 s
    // ahead of AIRA, is held on time 120 s (the rate-30 gap) behind AIRA's held STA. AIRA crosses
    // at its ETA, 60 s before AIRB's STA, inside AIRB's gap, and is flagged. AIRB keeps its STA.
    let state = state_with(pool.clone(), trailing_pair()).await;
    let id = corridor_fca(&pool, 30).await;
    let first = traffic(&state, &id).await;
    let eta_a = ms(&row(&first, "AIRA")["eta"]);
    assert!(
        ms(&row(&first, "AIRB")["eta"]) < eta_a - 5_000,
        "AIRB crosses ahead, so a hold after AIRA's ETA keeps it on time: {} vs {eta_a}",
        ms(&row(&first, "AIRB")["eta"])
    );
    seed_held(&pool, &id, "AIRB", eta_a + 60_000).await;
    seed_held(&pool, &id, "AIRA", eta_a - 60_000).await;

    let rows = traffic(&state, &id).await;
    let a = row(&rows, "AIRA");
    let b = row(&rows, "AIRB");
    let eta_now = ms(&a["eta"]);
    assert!(
        (eta_now - eta_a).abs() < 5_000,
        "the fixture's ETA is stable to a few seconds"
    );
    assert_eq!(
        ms(&b["cross_time"]),
        eta_a + 60_000,
        "the on-time neighbor keeps its STA"
    );
    assert_eq!(b["slot_conflict"], false);
    assert_eq!(b["late_sec"], 0);
    assert_eq!(
        ms(&a["cross_time"]),
        eta_now,
        "the late one crosses at its ETA"
    );
    assert_eq!(a["late_sec"], (eta_now - (eta_a - 60_000)) / 1000);
    assert_eq!(a["slot_conflict"], true, "inside AIRB's gap");
    assert_eq!(a["sta_frozen"], true);
    assert_eq!(
        held_rows(&pool, &id).await,
        vec![
            ("AIRA".to_string(), eta_a - 60_000),
            ("AIRB".to_string(), eta_a + 60_000)
        ],
        "the read changed no hold"
    );
}

/// An enabled corridor FCA at `rate` per hour with a 30 min horizon, and its id.
async fn corridor_fca_at_rate(pool: &PgPool, rate: i32) -> String {
    sqlx::query_scalar(
        "insert into flow.fca (name, color, artcc, points, dir, mode, rate, mit, enabled, freeze_min) \
         values ('FRZ', '#fff', 'ZDC', $1, 'any', 'rate', $2, 0, true, 30) returning id",
    )
    .bind(json!([[39.5, -75.6], [39.5, -74.0]]))
    .bind(rate)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Through the traffic route, a held flight seconds late whose slot a held neighbor's gap pushed
/// past its ETA crosses at that slot, unflagged, not at its ETA inside the neighbor's gap.
#[sqlx::test]
async fn a_late_held_flight_behind_a_held_neighbors_gap_crosses_at_its_slot(pool: PgPool) {
    // Rate 60 (60 s). AIRB, about 50 s ahead of AIRA, is held on time 5 s after its ETA. AIRA is
    // held 10 s after AIRB's STA, inside AIRB's gap, so its slot is AIRB's STA + 60 s. AIRA's ETA
    // is past its held STA (late) but short of that slot: it crosses at the slot.
    let state = state_with(pool.clone(), trailing_pair()).await;
    let id = corridor_fca_at_rate(&pool, 60).await;
    let first = traffic(&state, &id).await;
    let eta_a = ms(&row(&first, "AIRA")["eta"]);
    let eta_b = ms(&row(&first, "AIRB")["eta"]);
    let sta_b = eta_b + 5_000;
    let sta_a = sta_b + 10_000;
    let slot_a = sta_b + 60_000;
    assert!(
        eta_a > sta_a + 5_000 && eta_a < slot_a - 5_000,
        "fixture: AIRA's ETA {eta_a} sits between its held STA {sta_a} and its slot {slot_a}"
    );
    seed_held(&pool, &id, "AIRB", sta_b).await;
    seed_held(&pool, &id, "AIRA", sta_a).await;

    let rows = traffic(&state, &id).await;
    let a = row(&rows, "AIRA");
    let b = row(&rows, "AIRB");
    assert_eq!(ms(&b["cross_time"]), sta_b, "AIRB keeps its STA");
    assert_eq!(b["late_sec"], 0);
    assert_eq!(
        ms(&a["cross_time"]),
        slot_a,
        "AIRA keeps the slot it held, not its ETA in AIRB's gap"
    );
    assert!(a["late_sec"].as_i64().unwrap() > 0, "AIRA runs late: {a}");
    assert_eq!(a["slot_conflict"], false);
}

/// Two passes of the hold pass: a flight that freezes while a held flight runs late is stored clear
/// of the late flight's reserved slot, and the next pass and read serve that STA unchanged.
#[sqlx::test]
async fn a_flight_held_beside_a_late_held_flight_keeps_its_sta_on_the_next_pass(pool: PgPool) {
    // Rate 120 (30 s). AIRA is held 10 s before AIRB's ETA, so AIRA runs late and crosses at its
    // ETA, about 50 s after AIRB's. AIRB is not held yet; its ETA is inside the slot AIRA held,
    // which stays reserved, so the pass stores AIRB at AIRA's held STA + 30 s.
    let state = state_with(pool.clone(), trailing_pair()).await;
    let id = corridor_fca_at_rate(&pool, 120).await;
    let first = traffic(&state, &id).await;
    let eta_a = ms(&row(&first, "AIRA")["eta"]);
    let eta_b = ms(&row(&first, "AIRB")["eta"]);
    // AIRB is stored at its ETA + 20 s and must still sit a full 30 s gap before AIRA's ETA.
    assert!(
        eta_a - eta_b >= 50_000,
        "fixture: AIRA crosses at least 50 s after AIRB: {eta_b} vs {eta_a}"
    );
    let sta_a = eta_b - 10_000;
    seed_held(&pool, &id, "AIRA", sta_a).await;

    sync_fca_holds_once(&state, Utc::now()).await.unwrap();
    assert_eq!(
        held_rows(&pool, &id).await,
        vec![
            ("AIRA".to_string(), sta_a),
            ("AIRB".to_string(), sta_a + 30_000)
        ],
        "AIRB is stored clear of AIRA's reserved slot, not at its ETA"
    );
    sync_fca_holds_once(&state, Utc::now()).await.unwrap();
    let rows = traffic(&state, &id).await;
    let b = row(&rows, "AIRB");
    assert_eq!(b["sta_frozen"], true);
    assert_eq!(
        ms(&b["cross_time"]),
        sta_a + 30_000,
        "the next pass and read serve AIRB's STA unchanged"
    );
    assert_eq!(b["late_sec"], 0);
    assert_eq!(b["slot_conflict"], false);
    let a = row(&rows, "AIRA");
    assert_eq!(
        ms(&a["cross_time"]),
        ms(&a["eta"]),
        "AIRA crosses at its ETA"
    );
    assert!(a["late_sec"].as_i64().unwrap() > 0);
    assert_eq!(
        held_rows(&pool, &id).await,
        vec![
            ("AIRA".to_string(), sta_a),
            ("AIRB".to_string(), sta_a + 30_000)
        ],
        "the second pass changed no hold"
    );
}

/// The hold pass waits on `aircraft_profiles_loaded`; a profile write reloads the table, so it
/// marks the profiles loaded too, through the real route.
#[sqlx::test]
async fn a_profile_write_marks_the_profiles_loaded(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    assert!(
        !state.aircraft_profiles_loaded.load(Ordering::Relaxed),
        "a fresh state starts unloaded"
    );
    let cookie = national(&pool, "flow.aircraft_profiles.update").await;
    let profile = json!({ "name": "Test jet", "climb_ias_lo": 250.0, "climb_ias_hi": 290.0,
        "climb_fpm_lo": 1500.0, "climb_fpm_hi": 2500.0, "service_ceiling_ft": 41000.0,
        "desc_ias_hi": 290.0, "desc_ias_lo": 250.0, "desc_fpm": 2000.0 });
    let (status, body) = call(
        &state,
        Method::PUT,
        "/api/v1/flow/aircraft-profiles/type/T607",
        &cookie,
        Some(profile),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(state.aircraft_profiles_loaded.load(Ordering::Relaxed));
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

/// A row exactly `HELD_STA_RETAIN_MS` past its STA is still held by the read and kept by the sync's
/// stale prune; one millisecond older is neither.
#[sqlx::test]
async fn a_held_row_at_the_retention_edge_is_kept_and_one_ms_older_is_not(pool: PgPool) {
    let id = corridor_fca(&pool, 30).await;
    let now = Utc::now();
    let edge = now.timestamp_millis() - flow_repo::HELD_STA_RETAIN_MS;
    seed_held(&pool, &id, "EDGE", edge).await;
    seed_held(&pool, &id, "PAST", edge - 1).await;

    let listed = flow_repo::list_held_stas(&pool, std::slice::from_ref(&id), now)
        .await
        .unwrap();
    assert_eq!(
        listed[&id],
        HashMap::from([("EDGE".to_string(), edge)]),
        "the read holds the edge row only"
    );

    let store = [("NEW1".to_string(), now.timestamp_millis() + 600_000)];
    let sync = flow_repo::HeldStaSync {
        store: &store,
        drop: &[],
    };
    let at = updated_at(&pool, &id).await;
    flow_repo::sync_held_stas(&pool, &id, at, &sync, now)
        .await
        .unwrap();
    assert_eq!(
        held_rows(&pool, &id).await,
        vec![("EDGE".to_string(), edge), ("NEW1".to_string(), store[0].1)],
        "the sync prunes the older row and keeps the edge row"
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
async fn sync_held_stas_stores_nothing_for_a_pass_that_raced_an_edit(pool: PgPool) {
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

/// A replica that has just started has no feed snapshot, then a snapshot before the airport table
/// loads, then no winds or aircraft profiles yet. No such tick may read the empty board as every
/// held flight having left, or store an STA metered on still air or default profiles.
#[sqlx::test]
async fn the_pass_keeps_every_hold_until_its_inputs_are_loaded(pool: PgPool) {
    let state = state_with(pool.clone(), vec![airborne("AIR1")]).await;
    let id = corridor_fca(&pool, 30).await;
    let sta = Utc::now().timestamp_millis() + 600_000;
    seed_held(&pool, &id, "AIR1", sta).await;
    seed_held(&pool, &id, "GONE", sta).await;
    let kept = vec![("AIR1".to_string(), sta), ("GONE".to_string(), sta)];
    // As at startup: both loaders registered and not finished, and the airport load failed (a skip,
    // not an error, while the table is empty).
    state
        .jobs
        .register(crate::jobs::WINDS_REFRESH_JOB, "winds", Some(3600), false);
    state.jobs.register(
        crate::jobs::AIRCRAFT_PROFILES_REFRESH_JOB,
        "profiles",
        Some(300),
        true,
    );
    state
        .jobs
        .register("airports_refresh", "airports", Some(3600), false);
    state.jobs.finish("airports_refresh", false, "fetch failed");

    let snapshot = state.feed.write().await.snapshot.take();
    let tick = sync_fca_holds_once(&state, Utc::now()).await.unwrap();
    assert_eq!(tick, "skipped: feed or airports not loaded yet");
    assert_eq!(
        held_rows(&pool, &id).await,
        kept,
        "no snapshot: every hold kept"
    );

    let airports = {
        let mut feed = state.feed.write().await;
        feed.snapshot = snapshot;
        std::mem::take(&mut feed.airports)
    };
    let tick = sync_fca_holds_once(&state, Utc::now()).await.unwrap();
    assert_eq!(tick, "skipped: feed or airports not loaded yet");
    assert_eq!(
        held_rows(&pool, &id).await,
        kept,
        "no airports: every hold kept"
    );
    state.feed.write().await.airports = airports;

    let winds_at = state.winds_refreshed.swap(0, Ordering::Relaxed);
    let tick = sync_fca_holds_once(&state, Utc::now()).await.unwrap();
    assert_eq!(tick, "skipped: winds not loaded yet");
    assert_eq!(
        held_rows(&pool, &id).await,
        kept,
        "no winds: every hold kept"
    );
    state.winds_refreshed.store(winds_at, Ordering::Relaxed);

    state
        .aircraft_profiles_loaded
        .store(false, Ordering::Relaxed);
    let tick = sync_fca_holds_once(&state, Utc::now()).await.unwrap();
    assert_eq!(tick, "skipped: aircraft profiles not loaded yet");
    assert_eq!(
        held_rows(&pool, &id).await,
        kept,
        "no profiles: every hold kept"
    );
    state
        .aircraft_profiles_loaded
        .store(true, Ordering::Relaxed);

    let tick = sync_fca_holds_once(&state, Utc::now()).await.unwrap();
    assert_eq!(tick, "synced");
    assert_eq!(
        held_rows(&pool, &id).await,
        vec![("AIR1".to_string(), sta)],
        "positive control: once every input is loaded, the pass drops the flight that left"
    );
}

/// A loader that has run and failed with nothing loaded fails the pass instead of skipping it, so
/// the job registry shows it, and still stores and drops nothing.
#[sqlx::test]
async fn the_pass_fails_while_a_failed_loader_has_left_its_input_empty(pool: PgPool) {
    let state = state_with(pool.clone(), vec![airborne("AIR1")]).await;
    let id = corridor_fca(&pool, 30).await;
    let sta = Utc::now().timestamp_millis() + 600_000;
    seed_held(&pool, &id, "GONE", sta).await;
    let kept = vec![("GONE".to_string(), sta)];

    let winds_at = state.winds_refreshed.swap(0, Ordering::Relaxed);
    state
        .jobs
        .register(crate::jobs::WINDS_REFRESH_JOB, "winds", Some(3600), false);
    state
        .jobs
        .finish(crate::jobs::WINDS_REFRESH_JOB, false, "fetch failed");
    let err = sync_fca_holds_once(&state, Utc::now()).await.unwrap_err();
    assert!(err.contains("winds"), "{err}");
    assert_eq!(
        held_rows(&pool, &id).await,
        kept,
        "a failed winds load changes nothing"
    );
    state.winds_refreshed.store(winds_at, Ordering::Relaxed);
    state
        .jobs
        .finish(crate::jobs::WINDS_REFRESH_JOB, true, "loaded");

    state
        .aircraft_profiles_loaded
        .store(false, Ordering::Relaxed);
    state.jobs.register(
        crate::jobs::AIRCRAFT_PROFILES_REFRESH_JOB,
        "profiles",
        Some(300),
        true,
    );
    state.jobs.finish(
        crate::jobs::AIRCRAFT_PROFILES_REFRESH_JOB,
        false,
        "load failed",
    );
    let err = sync_fca_holds_once(&state, Utc::now()).await.unwrap_err();
    assert!(err.contains("aircraft profiles"), "{err}");
    assert_eq!(
        held_rows(&pool, &id).await,
        kept,
        "a failed profiles load changes nothing"
    );

    state
        .aircraft_profiles_loaded
        .store(true, Ordering::Relaxed);
    assert_eq!(
        sync_fca_holds_once(&state, Utc::now()).await.unwrap(),
        "synced"
    );
    assert!(
        held_rows(&pool, &id).await.iter().all(|(c, _)| c != "GONE"),
        "positive control: with both loaded the pass runs and drops the flight that left"
    );
}

/// A replica running without a database has no holds to keep, so the pass succeeds as a no-op
/// instead of failing every tick in the job registry.
#[tokio::test]
async fn without_a_database_the_pass_succeeds_and_does_nothing() {
    let state = crate::state::AppState::without_db();
    assert_eq!(
        sync_fca_holds_once(&state, Utc::now()).await,
        Ok("no database")
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
        "a 1 min horizon does not freeze a flight 2 min out, as a larger unit would"
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

/// A Set two minutes out on a parked flight: its release time is the first legal slot from its
/// ETA less the whole ground allowance (pushback, start-up and taxi) plus 2 min, so the ETA, which
/// counts that allowance, is already past it, while the wheels-up is still ahead. The Set's own response and the next traffic read both keep the release time with no
/// lateness, and the held airborne flight keeps its STA.
#[sqlx::test]
async fn a_set_release_waiting_for_its_wheels_up_is_not_late(pool: PgPool) {
    let state = state_with(pool.clone(), vec![airborne("AIR1"), parked("GND1")]).await;
    let id = corridor_fca(&pool, 30).await;
    // Held a minute after its ETA, so it stays on time while the clock moves (module doc).
    let air = ms(&row(&traffic(&state, &id).await, "AIR1")["cross_time"]) + 60_000;
    seed_held(&pool, &id, "AIR1", air).await;
    let cookie = national(&pool, "flow.fca.update").await;
    let ready = (Utc::now() + chrono::Duration::minutes(2))
        .format("%H%M")
        .to_string();

    let rows = release(&state, &cookie, &id, "GND1", json!({ "ready": ready })).await;
    let (cta, edct): (i64, i64) = sqlx::query_as(
        "select cta_ms, edct_ms from flow.fca_release where fca_id = $1 and callsign = 'GND1'",
    )
    .bind(&id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        edct > Utc::now().timestamp_millis(),
        "the wheels-up is still ahead"
    );
    for (label, rows) in [
        ("Set response", rows),
        ("traffic read", traffic(&state, &id).await),
    ] {
        let g = row(&rows, "GND1");
        assert!(
            ms(&g["eta"]) > cta,
            "{label}: the scenario needs the ETA past the release: {g}"
        );
        assert_eq!(
            ms(&g["cross_time"]),
            cta,
            "{label}: keeps its release time: {g}"
        );
        assert_eq!(g["late_sec"], 0, "{label}: not late before its wheels-up");
        assert_eq!(
            ms(&row(&rows, "AIR1")["cross_time"]),
            air,
            "{label}: AIR1 keeps its STA"
        );
    }
}

/// The stored release is moved so its wheels-up passed 120 s ago and its release time is 10 min
/// ahead. The traffic read times its lateness from the wheels-up: 120 s, where the release time
/// would give none. A re-release two minutes out then answers with the new wheels-up, still
/// ahead, so the response shows no lateness; the old wheels-up would show it late again.
#[sqlx::test]
async fn a_ground_release_runs_late_from_its_stored_wheels_up_and_a_re_release_takes_the_new_one(
    pool: PgPool,
) {
    let state = state_with(pool.clone(), vec![parked("GND1")]).await;
    let id = corridor_fca(&pool, 30).await;
    let cookie = national(&pool, "flow.fca.update").await;
    let in_2_min = (Utc::now() + chrono::Duration::minutes(2))
        .format("%H%M")
        .to_string();
    release(&state, &cookie, &id, "GND1", json!({ "ready": in_2_min })).await;
    let now = Utc::now().timestamp_millis();
    let (old_edct, old_cta) = (now - 120_000, now + 600_000);
    sqlx::query(
        "update flow.fca_release set edct_ms = $2, cta_ms = $3 \
         where fca_id = $1 and callsign = 'GND1'",
    )
    .bind(&id)
    .bind(old_edct)
    .bind(old_cta)
    .execute(&pool)
    .await
    .unwrap();

    let read = traffic(&state, &id).await;
    let g = row(&read, "GND1");
    let late = g["late_sec"].as_i64().unwrap();
    assert!(
        (120..130).contains(&late),
        "late from the wheels-up, not the release time: {g}"
    );
    assert!(
        ms(&g["cross_time"]) >= old_cta,
        "no earlier than its release: {g}"
    );

    let in_3_min = (Utc::now() + chrono::Duration::minutes(3))
        .format("%H%M")
        .to_string();
    let rows = release(&state, &cookie, &id, "GND1", json!({ "ready": in_3_min })).await;
    let (cta, edct): (i64, i64) = sqlx::query_as(
        "select cta_ms, edct_ms from flow.fca_release where fca_id = $1 and callsign = 'GND1'",
    )
    .bind(&id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        edct > Utc::now().timestamp_millis(),
        "the new wheels-up is ahead"
    );
    let g = row(&rows, "GND1");
    assert_eq!(
        g["late_sec"], 0,
        "the response reads the new wheels-up: {g}"
    );
    assert_eq!(ms(&g["cross_time"]), cta, "the new release time: {g}");
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

/// Two Sets for the same ready time: since #816 a Set searches for a slot, so the second lands one
/// rate-30 gap (120 s) plus RDY/SET's 15 s margin behind the first, and neither is left in a gap.
#[sqlx::test]
async fn a_second_set_for_the_same_time_is_spaced_behind_the_first(pool: PgPool) {
    let state = state_with(pool.clone(), vec![parked("GND1"), parked("GND2")]).await;
    let id = corridor_fca(&pool, 30).await;
    let cookie = national(&pool, "flow.fca.update").await;
    let ready = (Utc::now() + chrono::Duration::minutes(40))
        .format("%H%M")
        .to_string();

    let first = release(&state, &cookie, &id, "GND1", json!({ "ready": ready })).await;
    let g1 = ms(&row(&first, "GND1")["cross_time"]);
    assert_eq!(
        row(&first, "GND1")["slot_conflict"],
        false,
        "alone it is clear"
    );
    let rows = release(&state, &cookie, &id, "GND2", json!({ "ready": ready })).await;
    assert_eq!(
        ms(&row(&rows, "GND1")["cross_time"]),
        g1,
        "the first keeps its slot"
    );
    assert_eq!(
        ms(&row(&rows, "GND2")["cross_time"]),
        g1 + 135_000,
        "one 120 s gap plus the 15 s margin behind the first"
    );
    assert_eq!(row(&rows, "GND1")["slot_conflict"], false);
    assert_eq!(row(&rows, "GND2")["slot_conflict"], false);
}

/// One of AC3's two exceptions reaches the board: two releases 135 s apart are clear at rate 30, and once
/// the rate drops to 10 (a 360 s gap) the traffic route flags both, while the airborne flight
/// crossing minutes out stays clear.
#[sqlx::test]
async fn the_traffic_route_flags_a_release_left_inside_the_gap(pool: PgPool) {
    let state = state_with(
        pool.clone(),
        vec![parked("GND1"), parked("GND2"), airborne("AIR1")],
    )
    .await;
    let id = corridor_fca(&pool, 30).await;
    let cookie = national(&pool, "flow.fca.update").await;
    let ready = (Utc::now() + chrono::Duration::minutes(40))
        .format("%H%M")
        .to_string();
    release(&state, &cookie, &id, "GND1", json!({ "ready": ready })).await;
    release(&state, &cookie, &id, "GND2", json!({ "ready": ready })).await;
    let before = traffic(&state, &id).await;
    assert_eq!(
        ms(&row(&before, "GND2")["cross_time"]) - ms(&row(&before, "GND1")["cross_time"]),
        135_000
    );
    assert_eq!(
        row(&before, "GND1")["slot_conflict"],
        false,
        "clear at rate 30"
    );
    assert_eq!(
        row(&before, "GND2")["slot_conflict"],
        false,
        "clear at rate 30"
    );

    sqlx::query("update flow.fca set rate = 10 where id = $1")
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
    let after = traffic(&state, &id).await;
    assert_eq!(
        row(&after, "GND1")["slot_conflict"],
        true,
        "inside GND2's 360 s gap"
    );
    assert_eq!(
        row(&after, "GND2")["slot_conflict"],
        true,
        "inside GND1's 360 s gap"
    );
    assert_eq!(
        row(&after, "AIR1")["slot_conflict"],
        false,
        "an unheld, unreleased flight is never flagged"
    );
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

// --- the pass is the only thing that stores holds ---

/// A corridor FCA with every state the pass's metering and prune read set explicitly.
struct FcaRow<'a> {
    name: &'a str,
    rate: i32,
    freeze_min: i32,
    enabled: bool,
    deleted: bool,
    event: Option<(i64, &'a str)>,
}

impl Default for FcaRow<'_> {
    fn default() -> Self {
        Self {
            name: "FRZ",
            rate: 30,
            freeze_min: 30,
            enabled: true,
            deleted: false,
            event: None,
        }
    }
}

async fn fca_row(pool: &PgPool, f: FcaRow<'_>) -> String {
    sqlx::query_scalar(
        "insert into flow.fca (name, color, artcc, points, dir, mode, rate, mit, enabled, \
             freeze_min, deleted_at, event_id, event_status) \
         values ($1, '#fff', 'ZDC', $2, 'any', 'rate', $3, 0, $4, $5, \
             case when $6 then now() end, $7, $8) returning id",
    )
    .bind(f.name)
    .bind(json!([[39.5, -75.6], [39.5, -74.0]]))
    .bind(f.rate)
    .bind(f.enabled)
    .bind(f.freeze_min)
    .bind(f.deleted)
    .bind(f.event.map(|(id, _)| id))
    .bind(f.event.map(|(_, status)| status))
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn seed_event(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "insert into events.event (id, title, start_time, end_time) \
         values (817, 'E', now(), now() + interval '1 hour') returning id",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

/// 60 s ahead of the wall clock: see the module doc.
fn pass_now() -> chrono::DateTime<Utc> {
    Utc::now() + chrono::Duration::seconds(60)
}

/// AIRB sits further down the track than AIR1's spot, so it crosses first.
fn trailing_pair() -> Vec<Pilot> {
    vec![
        airborne("AIRA"),
        Pilot {
            latitude: 40.38,
            longitude: -74.20,
            ..airborne("AIRB")
        },
    ]
}

#[sqlx::test]
async fn a_traffic_read_signed_out_or_in_writes_no_held_row(pool: PgPool) {
    let state = state_with(pool.clone(), vec![airborne("AIR1")]).await;
    let id = corridor_fca(&pool, 30).await;

    let rows = traffic(&state, &id).await;
    assert_eq!(
        row(&rows, "AIR1")["sta_frozen"],
        true,
        "positive control: the anonymous read metered AIR1 inside the horizon"
    );
    assert!(held_rows(&pool, &id).await.is_empty(), "anonymous read");

    let cookie = national(&pool, "flow.fca.read").await;
    let (status, body) = call(
        &state,
        Method::GET,
        &format!("/api/v1/flow/fcas/{id}/traffic"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(row(body.as_array().unwrap(), "AIR1")["sta_frozen"], true);
    assert!(held_rows(&pool, &id).await.is_empty(), "signed-in read");

    sync_fca_holds_once(&state, pass_now()).await.unwrap();
    let held = held_rows(&pool, &id).await;
    let rows = traffic(&state, &id).await;
    assert_eq!(
        held,
        vec![("AIR1".to_string(), ms(&row(&rows, "AIR1")["cross_time"]))],
        "positive control: the pass stores the flight the reads did not"
    );
}

#[sqlx::test]
async fn the_pass_stores_for_live_fcas_and_prunes_every_unmetered_fca(pool: PgPool) {
    let state = state_with(pool.clone(), vec![airborne("AIR1")]).await;
    let event = seed_event(&pool).await;
    let live = fca_row(&pool, FcaRow::default()).await;
    let published = fca_row(
        &pool,
        FcaRow {
            event: Some((event, "published")),
            ..Default::default()
        },
    )
    .await;
    let unmetered = [
        (
            "disabled",
            FcaRow {
                enabled: false,
                ..Default::default()
            },
        ),
        (
            "deleted",
            FcaRow {
                deleted: true,
                ..Default::default()
            },
        ),
        (
            "planned event",
            FcaRow {
                event: Some((event, "planned")),
                ..Default::default()
            },
        ),
        (
            "archived event",
            FcaRow {
                event: Some((event, "archived")),
                ..Default::default()
            },
        ),
    ];
    let seeded = Utc::now().timestamp_millis() + 10 * MIN;
    seed_held(&pool, &published, "AIR1", seeded).await;
    let mut pruned = Vec::new();
    for (label, f) in unmetered {
        let id = fca_row(&pool, f).await;
        seed_held(&pool, &id, "AIR1", seeded).await;
        pruned.push((label, id));
    }

    sync_fca_holds_once(&state, pass_now()).await.unwrap();

    let stored = held_rows(&pool, &live).await;
    let rows = traffic(&state, &live).await;
    assert_eq!(
        stored,
        vec![("AIR1".to_string(), ms(&row(&rows, "AIR1")["cross_time"]))],
        "the live FCA is held this tick"
    );
    assert_eq!(
        held_rows(&pool, &published).await,
        vec![("AIR1".to_string(), seeded)],
        "a published event FCA is metered and keeps its first STA"
    );
    for (label, id) in &pruned {
        assert!(
            held_rows(&pool, id).await.is_empty(),
            "the {label} FCA's holds are pruned"
        );
    }
}

#[sqlx::test]
async fn one_fcas_failed_sync_does_not_stop_the_pass(pool: PgPool) {
    let state = state_with(pool.clone(), vec![airborne("AIR1")]).await;
    // `list_fcas` orders by name, so the failing FCA is metered first.
    let bad = fca_row(
        &pool,
        FcaRow {
            name: "AAA",
            ..Default::default()
        },
    )
    .await;
    let good = fca_row(
        &pool,
        FcaRow {
            name: "ZZZ",
            ..Default::default()
        },
    )
    .await;
    let off = fca_row(
        &pool,
        FcaRow {
            name: "OFF",
            enabled: false,
            ..Default::default()
        },
    )
    .await;
    seed_held(&pool, &off, "AIR1", Utc::now().timestamp_millis()).await;
    sqlx::query(
        "create function public.fail_held_sta() returns trigger language plpgsql \
         as $$ begin raise exception 'injected'; end $$",
    )
    .execute(&pool)
    .await
    .unwrap();
    // `bad` is an id this test inserted, not input.
    sqlx::query(&format!(
        "create trigger fail_held before insert on flow.fca_held_sta for each row \
         when (new.fca_id = '{bad}') execute function public.fail_held_sta()"
    ))
    .execute(&pool)
    .await
    .unwrap();

    let result = sync_fca_holds_once(&state, pass_now()).await;

    let err = result.expect_err("the failed FCA is reported");
    assert!(
        err.contains(&format!("{bad} (store holds)")),
        "the error names the failed FCA and its step: {err}"
    );
    assert!(!err.contains(&good), "and only it: {err}");
    assert!(held_rows(&pool, &bad).await.is_empty());
    assert_eq!(
        held_rows(&pool, &good).await.len(),
        1,
        "the FCA after the failure is still held"
    );
    assert!(
        held_rows(&pool, &off).await.is_empty(),
        "the prune still ran"
    );
}

/// The pass's error names the step an FCA failed at, so a release read that fails is not
/// reported as a failed store.
#[sqlx::test]
async fn a_failed_release_read_is_reported_as_that_step(pool: PgPool) {
    let state = state_with(pool.clone(), vec![airborne("AIR1")]).await;
    let id = corridor_fca(&pool, 30).await;
    sqlx::query("alter table flow.fca_release rename to fca_release_hidden")
        .execute(&pool)
        .await
        .unwrap();

    let err = sync_fca_holds_once(&state, pass_now())
        .await
        .expect_err("the release read fails");

    assert!(
        err.contains(&format!("{id} (load releases)")),
        "the error names the FCA and the step: {err}"
    );
    assert!(!err.contains("store holds"), "{err}");
    assert!(held_rows(&pool, &id).await.is_empty(), "nothing stored");
}

#[sqlx::test]
async fn a_held_flight_stays_frozen_up_to_two_minutes_past_the_horizon(pool: PgPool) {
    let state = state_with(pool.clone(), vec![airborne("AIR1")]).await;
    let now = Utc::now();
    let now_ms = now.timestamp_millis();
    // A rate-0 probe with the widest horizon stores AIR1 at its unmetered ETA.
    let probe = fca_row(
        &pool,
        FcaRow {
            rate: 0,
            freeze_min: 120,
            ..Default::default()
        },
    )
    .await;
    sync_fca_holds_once(&state, now).await.unwrap();
    let eta = held_rows(&pool, &probe).await[0].1;
    let out = eta - now_ms;
    assert!(out > 4 * MIN, "fixture: AIR1 is {out} ms out");
    // The smallest whole-minute horizon a 2 min margin still covers, and the one below it.
    let inside = (out - 120_000 + MIN - 1) / MIN;
    let beyond = inside - 1;
    assert!(inside * MIN < out && out <= inside * MIN + 120_000 && inside <= 120);
    assert!(out > beyond * MIN + 120_000);
    let kept = fca_row(
        &pool,
        FcaRow {
            name: "KEPT",
            freeze_min: inside as i32,
            ..Default::default()
        },
    )
    .await;
    let fresh = fca_row(
        &pool,
        FcaRow {
            name: "FRESH",
            freeze_min: inside as i32,
            ..Default::default()
        },
    )
    .await;
    let dropped = fca_row(
        &pool,
        FcaRow {
            name: "DROPPED",
            freeze_min: beyond as i32,
            ..Default::default()
        },
    )
    .await;
    seed_held(&pool, &kept, "AIR1", eta).await;
    seed_held(&pool, &dropped, "AIR1", eta).await;

    sync_fca_holds_once(&state, now).await.unwrap();

    assert_eq!(
        held_rows(&pool, &kept).await,
        vec![("AIR1".to_string(), eta)],
        "past the {inside} min horizon, inside the margin: kept"
    );
    assert!(
        held_rows(&pool, &fresh).await.is_empty(),
        "no stored STA: the plain {inside} min horizon applies"
    );
    assert!(
        held_rows(&pool, &dropped).await.is_empty(),
        "past the {beyond} min horizon plus the margin: dropped"
    );
}

async fn put_order(state: &AppState, cookie: &str, id: &str, order: &[&str]) {
    let status = send(
        state,
        Method::PUT,
        &format!("/api/v1/flow/fcas/{id}/order"),
        cookie,
        Some(json!({ "order": order })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "order {order:?}");
}

#[sqlx::test]
async fn a_real_reorder_clears_the_holds_and_the_pass_re_holds_in_the_new_order(pool: PgPool) {
    // Rate 30 is a 120 s gap. B crosses first; B is held a minute after its ETA and A one gap
    // behind B. Dragging A first re-meters from the ETAs: A at its ETA, B one gap behind A. Kept
    // holds would anchor A at its old STA and push B a gap behind that.
    let state = state_with(pool.clone(), trailing_pair()).await;
    let id = corridor_fca(&pool, 30).await;
    let probe = fca_row(
        &pool,
        FcaRow {
            name: "PROBE",
            rate: 0,
            ..Default::default()
        },
    )
    .await;
    let now = pass_now();
    sync_fca_holds_once(&state, now).await.unwrap();
    let etas = held_rows(&pool, &probe).await;
    let (eta_a, eta_b) = (etas[0].1, etas[1].1);
    assert!(
        eta_b < eta_a && eta_a < eta_b + 180_000,
        "fixture: B first, A within 3 min of it ({eta_a}, {eta_b})"
    );
    sqlx::query("delete from flow.fca_held_sta where fca_id = $1")
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
    let (held_b, held_a) = (eta_b + MIN, eta_b + 3 * MIN);
    seed_held(&pool, &id, "AIRB", held_b).await;
    seed_held(&pool, &id, "AIRA", held_a).await;
    let rows = traffic(&state, &id).await;
    assert_eq!(ms(&row(&rows, "AIRB")["cross_time"]), held_b, "control");
    assert_eq!(ms(&row(&rows, "AIRA")["cross_time"]), held_a, "control");

    let cookie = national(&pool, "flow.fca.update").await;
    put_order(&state, &cookie, &id, &["AIRA", "AIRB"]).await;
    assert!(
        held_rows(&pool, &id).await.is_empty(),
        "the reorder clears the old order's holds"
    );
    assert_eq!(held_rows(&pool, &probe).await, etas, "and no other FCA's");

    sync_fca_holds_once(&state, now).await.unwrap();
    assert_eq!(
        held_rows(&pool, &id).await,
        vec![
            ("AIRA".to_string(), eta_a),
            ("AIRB".to_string(), eta_a + 120_000)
        ],
        "re-held from the ETAs, not ({held_a}, {})",
        held_a + 120_000
    );
    let rows = traffic(&state, &id).await;
    let (a, b) = (row(&rows, "AIRA"), row(&rows, "AIRB"));
    assert_eq!((ms(&a["cross_time"]), &a["seq"]), (eta_a, &json!(1)));
    assert_eq!(
        (ms(&b["cross_time"]), &b["seq"]),
        (eta_a + 120_000, &json!(2))
    );
}

#[sqlx::test]
async fn a_manual_order_save_that_changes_nothing_keeps_the_holds(pool: PgPool) {
    let state = state_with(pool.clone(), trailing_pair()).await;
    let manual = corridor_fca(&pool, 30).await;
    let auto = corridor_fca(&pool, 30).await;
    let stored: (Vec<String>, bool) =
        sqlx::query_as("select manual_order, manual_seq from flow.fca where id = $1")
            .bind(&auto)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored, (vec![], false), "fixture: auto by default");
    let cookie = national(&pool, "flow.fca.update").await;
    let now = pass_now();

    sync_fca_holds_once(&state, now).await.unwrap();
    let auto_held = held_rows(&pool, &auto).await;
    assert_eq!(auto_held.len(), 2, "positive control: both held");
    put_order(&state, &cookie, &auto, &[]).await;
    assert_eq!(
        held_rows(&pool, &auto).await,
        auto_held,
        "an auto FCA saved as auto keeps its holds"
    );

    put_order(&state, &cookie, &manual, &["AIRA", "AIRB"]).await;
    assert!(
        held_rows(&pool, &manual).await.is_empty(),
        "auto to manual clears"
    );
    sync_fca_holds_once(&state, now).await.unwrap();
    let before = held_rows(&pool, &manual).await;
    assert_eq!(before.len(), 2);
    put_order(&state, &cookie, &manual, &["AIRA", "AIRB"]).await;
    assert_eq!(
        held_rows(&pool, &manual).await,
        before,
        "an identical save keeps the holds"
    );

    put_order(&state, &cookie, &manual, &["AIRB", "AIRA"]).await;
    assert!(
        held_rows(&pool, &manual).await.is_empty(),
        "a same-size swap clears"
    );
    assert_eq!(held_rows(&pool, &auto).await, auto_held, "no other FCA's");

    sync_fca_holds_once(&state, now).await.unwrap();
    assert_eq!(held_rows(&pool, &manual).await.len(), 2);
    put_order(&state, &cookie, &manual, &[]).await;
    assert!(
        held_rows(&pool, &manual).await.is_empty(),
        "manual back to auto clears"
    );
}

/// Every `UpsertFcaRequest` field explicit, matching what `corridor_fca` stores where it can.
fn full_body() -> Value {
    json!({
        "name": "FRZ", "color": "#efc14d", "artcc": "ZDC",
        "points": [[39.5, -75.6], [39.5, -74.0]], "dests": [], "origins": [], "fixes": [],
        "scope": [], "min_fl": null, "max_fl": null, "dir": "any", "mode": "rate",
        "rate": 30, "mit": 15, "freeze_min": 30, "enabled": true
    })
}

async fn put_body(state: &AppState, cookie: &str, id: &str, body: Value, label: &str) {
    let status = send(
        state,
        Method::PUT,
        &format!("/api/v1/flow/fcas/{id}"),
        cookie,
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{label}");
}

async fn stored_field(pool: &PgPool, id: &str, field: &str) -> Value {
    let fca = flow_repo::get_fca(pool, id).await.unwrap().unwrap();
    serde_json::to_value(fca).unwrap()[field].clone()
}

/// A second FCA whose geometry and rate differ from `full_body()`, so the edit's clear has a row
/// that does not match the request and must not be compared against it.
async fn other_fca(pool: &PgPool) -> String {
    sqlx::query_scalar(
        "insert into flow.fca (name, color, artcc, points, dir, mode, rate, mit, enabled, freeze_min) \
         values ('OTHER', '#fff', 'ZNY', $1, 'any', 'rate', 10, 0, true, 45) returning id",
    )
    .bind(json!([[41.0, -75.0], [41.0, -73.0]]))
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test]
async fn an_edit_to_name_color_artcc_or_enabled_keeps_the_held_stas(pool: PgPool) {
    let state = state_with(pool.clone(), vec![]).await;
    let id = corridor_fca(&pool, 30).await;
    let other = other_fca(&pool).await;
    let cookie = national(&pool, "flow.fca.update").await;
    put_body(&state, &cookie, &id, full_body(), "base").await;
    let sta = Utc::now().timestamp_millis() + 10 * MIN;
    seed_held(&pool, &id, "AIR1", sta).await;
    seed_held(&pool, &other, "OTH1", sta).await;

    // A client that predates `freeze_min` sends no horizon; with nothing else changed the stored
    // horizon is compared, so the holds stay.
    let mut no_horizon = full_body();
    no_horizon.as_object_mut().unwrap().remove("freeze_min");
    put_body(&state, &cookie, &id, no_horizon, "no freeze_min").await;
    assert_eq!(
        held_rows(&pool, &id).await,
        vec![("AIR1".to_string(), sta)],
        "a save omitting freeze_min with nothing changed keeps the held STAs"
    );

    for (field, value) in [
        ("name", json!("FRZ RENAMED")),
        ("color", json!("#ffffff")),
        ("artcc", json!("ZNY")),
        ("enabled", json!(false)),
    ] {
        let mut changed = full_body();
        changed[field] = value.clone();
        put_body(&state, &cookie, &id, changed, field).await;
        assert_eq!(
            stored_field(&pool, &id, field).await,
            value,
            "{field} was stored"
        );
        assert_eq!(
            held_rows(&pool, &id).await,
            vec![("AIR1".to_string(), sta)],
            "changing {field} alone keeps the held STAs"
        );
        put_body(&state, &cookie, &id, full_body(), field).await;
    }

    let mut rate = full_body();
    rate["rate"] = json!(20);
    put_body(&state, &cookie, &id, rate, "rate").await;
    assert!(
        held_rows(&pool, &id).await.is_empty(),
        "positive control: a metering change clears"
    );
    assert_eq!(
        held_rows(&pool, &other).await,
        vec![("OTH1".to_string(), sta)],
        "another FCA's holds survive every edit"
    );
}

#[sqlx::test]
async fn an_edit_to_any_one_metering_field_clears_the_held_stas(pool: PgPool) {
    let state = state_with(pool.clone(), vec![]).await;
    let id = corridor_fca(&pool, 30).await;
    let cookie = national(&pool, "flow.fca.update").await;
    let sta = Utc::now().timestamp_millis() + 10 * MIN;

    for (field, value) in [
        ("points", json!([[39.4, -75.6], [39.4, -74.0]])),
        ("dests", json!(["KDCA"])),
        ("origins", json!(["KJFK"])),
        ("fixes", json!(["SIE"])),
        ("scope", json!(["ZDC_01"])),
        ("min_fl", json!(100)),
        ("max_fl", json!(450)),
        ("dir", json!("S")),
        ("mode", json!("mit")),
        ("rate", json!(20)),
        ("mit", json!(20)),
        ("freeze_min", json!(45)),
    ] {
        put_body(&state, &cookie, &id, full_body(), field).await;
        seed_held(&pool, &id, "AIR1", sta).await;
        put_body(&state, &cookie, &id, full_body(), field).await;
        assert_eq!(
            held_rows(&pool, &id).await,
            vec![("AIR1".to_string(), sta)],
            "{field}: an unchanged save keeps the held STAs"
        );
        let mut changed = full_body();
        changed[field] = value.clone();
        put_body(&state, &cookie, &id, changed, field).await;
        assert_eq!(
            stored_field(&pool, &id, field).await,
            value,
            "{field} was stored"
        );
        assert!(
            held_rows(&pool, &id).await.is_empty(),
            "changing {field} alone clears the held STAs"
        );
    }
}

#[sqlx::test]
async fn the_pass_drops_the_hold_of_a_flight_released_by_rdy_or_set(pool: PgPool) {
    let mut pilots = trailing_pair();
    pilots.push(Pilot {
        latitude: 40.30,
        longitude: -74.30,
        ..airborne("AIRC")
    });
    let state = state_with(pool.clone(), pilots).await;
    let id = corridor_fca(&pool, 30).await;
    sync_fca_holds_once(&state, pass_now()).await.unwrap();
    let before = held_rows(&pool, &id).await;
    let held: Vec<&str> = before.iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(held, ["AIRA", "AIRB", "AIRC"], "fixture: all three held");
    let cookie = national(&pool, "flow.fca.update").await;

    release(&state, &cookie, &id, "AIRA", json!({})).await;
    let ready = (Utc::now() + chrono::Duration::minutes(40))
        .format("%H%M")
        .to_string();
    let rows = release(&state, &cookie, &id, "AIRB", json!({ "ready": ready })).await;
    assert_eq!(row(&rows, "AIRA")["released"], true, "RDY stands");
    assert_eq!(row(&rows, "AIRB")["released"], true, "SET stands");
    assert_eq!(
        held_rows(&pool, &id).await,
        before,
        "a release writes no hold"
    );

    sync_fca_holds_once(&state, pass_now()).await.unwrap();
    assert_eq!(
        held_rows(&pool, &id).await,
        vec![before[2].clone()],
        "the pass drops both released flights' holds; AIRC keeps its first STA"
    );
}

#[sqlx::test]
async fn the_flight_advisory_serves_the_held_sta(pool: PgPool) {
    let state = state_with(pool.clone(), vec![airborne("AIR1")]).await;
    let id = corridor_fca(&pool, 30).await;
    let eta = ms(&row(&traffic(&state, &id).await, "AIR1")["eta"]);
    let held = eta + 5 * MIN;
    seed_held(&pool, &id, "AIR1", held).await;

    let (status, body) = send_json(&state, Method::GET, "/api/v1/public/flight/AIR1", "").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["found"], true);
    let fcas = body["fcas"].as_array().unwrap();
    assert_eq!(fcas.len(), 1, "{body}");
    assert_eq!(fcas[0]["fca_id"], id.as_str());
    assert_eq!(
        ms(&fcas[0]["cross_time"]),
        held,
        "the held STA, not the ETA"
    );
    assert_eq!(
        held_rows(&pool, &id).await,
        vec![("AIR1".to_string(), held)],
        "the advisory only reads"
    );
}

#[sqlx::test]
async fn a_create_with_a_45_min_horizon_round_trips(pool: PgPool) {
    let state = state_with(pool.clone(), vec![]).await;
    let cookie = national(&pool, "flow.fca.update").await;
    let (status, body) = call(
        &state,
        Method::POST,
        "/api/v1/flow/fcas",
        &cookie,
        Some(json!({
            "name": "NEW", "artcc": "ZDC", "points": [[39.5, -75.6], [39.5, -74.0]],
            "mode": "rate", "rate": 20, "freeze_min": 45
        })),
    )
    .await;
    assert!(status.is_success(), "{status} {body}");
    let id = body["id"].as_str().unwrap();
    let fca = flow_repo::get_fca(&pool, id).await.unwrap().unwrap();
    assert_eq!(fca.freeze_min, 45, "not the default 30");
}
