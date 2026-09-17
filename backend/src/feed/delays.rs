//! Delay-leg collector. A background worker reads OIS's shared feed snapshot each cycle and records,
//! per completed flight, two timings into `stats.flight_leg` for the average-delay page:
//!   * **departure** — taxi-out: start-of-taxi (~7 kt) → wheels-up (>60 kt or a >100 ft climb).
//!   * **arrival**   — transit: crossing a ~40 NM entry ring → touchdown (slow, at the field).
//!
//! Each leg is tagged with the detected runway (heading vs the field's runway ends) and the filed
//! SID/STAR base name. Departure detection mirrors `feed/taxi.rs` (which powers the live Taxi
//! Monitor); arrivals are new. All state is in-memory and rebuilds on restart.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use super::FeedState;
use super::airports::{Airport, AirportDb};
use super::flow::{arrival_gate, gc_dist};
use super::runway::star_base;
use super::runway_db::RunwayDb;
use super::vatsim::VatsimData;
use crate::repos::stats::{self as repo, FlightLegRow};

// --- departure (taxi-out) tuning — matches feed/taxi.rs ---
const GS_START: i64 = 7; // kt — taxi has begun
const GS_STOP: i64 = 60; // kt — airborne
const ALT_CLIMB_FT: i64 = 100;
const DEP_PROX_NM: f64 = 15.0;
const MIN_TAXI_SEC: i64 = 3;
const MAX_TAXI_SEC: i64 = 60 * 60;

// --- arrival (entry → touchdown) tuning ---
const TRACK_START_NM: f64 = 60.0; // begin tracking an inbound within this of the field
const ENTRY_NM: f64 = 40.0; // the "airspace entry" ring
const LAND_GS: i64 = 40; // kt — slowed to a rollout/on the ground
const LAND_PROX_NM: f64 = 3.0; // touchdown must be this close to the field
const MIN_TRANSIT_SEC: i64 = 60;
const MAX_TRANSIT_SEC: i64 = 3 * 60 * 60;

const SESSION_MAX_AGE_MS: i64 = 3 * 60 * 60_000;
const RUNWAY_TOL_DEG: f64 = 30.0; // heading must be within this of a runway end to tag it
const COLLECT_SECS: u64 = 15;

#[derive(Clone, Copy, PartialEq)]
enum DepPhase {
    Watching,
    Rolling,
}

struct DepSession {
    dep: String,
    phase: DepPhase,
    first_seen_ms: i64,
    start_ms: Option<i64>,
    base_alt: i64,
}

struct ArrSession {
    arr: String,
    first_ms: i64,
    /// When the aircraft crossed the entry ring (only set for a genuine outside→inside crossing).
    entry_ms: Option<i64>,
    /// Last heading seen while airborne — used to tag the landing runway.
    last_hdg: i64,
}

#[derive(Default)]
pub struct DelayState {
    dep: HashMap<String, DepSession>,
    arr: HashMap<String, ArrSession>,
}

fn angle_diff(a: f64, b: f64) -> f64 {
    let d = (a - b).abs() % 360.0;
    if d > 180.0 { 360.0 - d } else { d }
}

/// The runway end at `icao` whose heading is closest to `heading` (within tolerance), else None.
pub(crate) fn nearest_runway(runways: &RunwayDb, icao: &str, heading: i64) -> Option<String> {
    let mut best: Option<(String, f64)> = None;
    for e in runways.ends_for(icao) {
        let d = angle_diff(heading as f64, e.hdg as f64);
        if best.as_ref().is_none_or(|(_, bd)| d < *bd) {
            best = Some((e.id, d));
        }
    }
    best.filter(|(_, d)| *d <= RUNWAY_TOL_DEG).map(|(id, _)| id)
}

/// The filed SID base name: the first route token that looks like a named procedure (≥4 leading
/// letters then a revision digit, e.g. `GLASR3` → `GLASR`), scanning only the front of the route so
/// enroute airways (`J146`) and fixes aren't mistaken for a departure procedure.
fn sid_of(route: &str) -> Option<String> {
    for raw in route
        .to_ascii_uppercase()
        .split([' ', '\t', '\n', '\r', '.', '/'])
        .take(2)
    {
        let tok: String = raw.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        let leading_alpha = tok.bytes().take_while(|b| b.is_ascii_alphabetic()).count();
        if leading_alpha >= 4 && tok.bytes().last().is_some_and(|b| b.is_ascii_digit()) {
            return Some(star_base(&tok));
        }
    }
    None
}

/// Advance the delay state machine with a fresh snapshot; returns completed legs to persist.
pub fn process(
    state: &mut DelayState,
    airports: &AirportDb,
    runways: &RunwayDb,
    data: &VatsimData,
    now: DateTime<Utc>,
) -> Vec<FlightLegRow> {
    let now_ms = now.timestamp_millis();
    let ts = |ms_ago: i64| now - chrono::Duration::milliseconds(ms_ago);
    let mut out = Vec::new();
    let mut seen_dep: HashSet<String> = HashSet::new();
    let mut seen_arr: HashSet<String> = HashSet::new();
    let mut dep_done: Vec<String> = Vec::new();
    let mut arr_done: Vec<String> = Vec::new();

    for p in &data.pilots {
        let Some(fp) = &p.flight_plan else { continue };
        let gs = p.groundspeed;
        let alt = p.altitude;
        let dep = fp.departure.to_ascii_uppercase();
        let arr = fp.arrival.to_ascii_uppercase();

        // --- Departure: watch it roll off its field, time start-of-taxi → wheels-up. ---
        if !dep.is_empty() {
            if let Some(s) = state.dep.get_mut(&p.callsign).filter(|s| s.dep == dep) {
                seen_dep.insert(p.callsign.clone());
                if s.phase == DepPhase::Watching && gs > GS_START {
                    s.phase = DepPhase::Rolling;
                    s.start_ms = Some(now_ms);
                    s.base_alt = alt;
                }
                if s.phase == DepPhase::Rolling
                    && (gs > GS_STOP || alt >= s.base_alt + ALT_CLIMB_FT)
                    && let Some(start) = s.start_ms
                {
                    // A very short observed roll means we caught it mid-taxi; fall back to first-seen.
                    let start_ms = if now_ms - start < 3_000 && s.first_seen_ms < start {
                        s.first_seen_ms
                    } else {
                        start
                    };
                    let dur = (now_ms - start_ms) / 1000;
                    if (MIN_TAXI_SEC..=MAX_TAXI_SEC).contains(&dur) {
                        out.push(FlightLegRow {
                            kind: "departure",
                            airport: dep.clone(),
                            callsign: p.callsign.clone(),
                            cid: p.cid,
                            aircraft: (!fp.aircraft_short.is_empty())
                                .then(|| fp.aircraft_short.clone()),
                            runway: nearest_runway(runways, &dep, p.heading),
                            procedure: sid_of(&fp.route),
                            start: ts(now_ms - start_ms),
                            end: now,
                            duration_sec: dur as i32,
                        });
                    }
                    dep_done.push(p.callsign.clone());
                }
            } else if let Some(&Airport {
                lat: dlat,
                lon: dlon,
                ..
            }) = airports.get(&dep)
            {
                // Start a session only for a genuine departure sitting at its field.
                let arriving_turnaround = gs <= GS_STOP
                    && airports.get(&arr).is_some_and(
                        |&Airport {
                             lat: alat,
                             lon: alon,
                             ..
                         }| {
                            gc_dist(p.latitude, p.longitude, alat, alon) < 5.0
                        },
                    );
                if gc_dist(p.latitude, p.longitude, dlat, dlon) <= DEP_PROX_NM
                    && !(gs > GS_STOP && alt > 500)
                    && !arriving_turnaround
                {
                    state.dep.insert(
                        p.callsign.clone(),
                        DepSession {
                            dep: dep.clone(),
                            phase: DepPhase::Watching,
                            first_seen_ms: now_ms,
                            start_ms: None,
                            base_alt: alt,
                        },
                    );
                    seen_dep.insert(p.callsign.clone());
                }
            }
        }

        // --- Arrival: time entry-ring crossing → touchdown at the destination field. ---
        if let Some(&Airport {
            lat: alat,
            lon: alon,
            ..
        }) = airports.get(&arr)
        {
            let dist = gc_dist(p.latitude, p.longitude, alat, alon);
            if let Some(s) = state.arr.get_mut(&p.callsign).filter(|s| s.arr == arr) {
                seen_arr.insert(p.callsign.clone());
                if gs > GS_STOP {
                    s.last_hdg = p.heading;
                    if s.entry_ms.is_none() && dist <= ENTRY_NM {
                        s.entry_ms = Some(now_ms); // crossed the ring (session started outside it)
                    }
                } else if let Some(entry) = s.entry_ms
                    && gs < LAND_GS
                    && dist <= LAND_PROX_NM
                {
                    let dur = (now_ms - entry) / 1000;
                    if (MIN_TRANSIT_SEC..=MAX_TRANSIT_SEC).contains(&dur) {
                        out.push(FlightLegRow {
                            kind: "arrival",
                            airport: arr.clone(),
                            callsign: p.callsign.clone(),
                            cid: p.cid,
                            aircraft: (!fp.aircraft_short.is_empty())
                                .then(|| fp.aircraft_short.clone()),
                            runway: nearest_runway(runways, &arr, s.last_hdg),
                            procedure: arrival_gate(&fp.route, &arr).map(|g| star_base(&g)),
                            start: ts(now_ms - entry),
                            end: now,
                            duration_sec: dur as i32,
                        });
                    }
                    arr_done.push(p.callsign.clone());
                }
            } else if gs > GS_STOP && dist > ENTRY_NM && dist <= TRACK_START_NM {
                // Airborne, inbound, still outside the ring — track so we catch the crossing.
                state.arr.insert(
                    p.callsign.clone(),
                    ArrSession {
                        arr: arr.clone(),
                        first_ms: now_ms,
                        entry_ms: None,
                        last_hdg: p.heading,
                    },
                );
                seen_arr.insert(p.callsign.clone());
            }
        }
    }

    for cs in dep_done {
        state.dep.remove(&cs);
    }
    for cs in arr_done {
        state.arr.remove(&cs);
    }
    // Drop sessions for flights that vanished or lingered too long.
    state
        .dep
        .retain(|cs, s| seen_dep.contains(cs) && now_ms - s.first_seen_ms < SESSION_MAX_AGE_MS);
    state
        .arr
        .retain(|cs, s| seen_arr.contains(cs) && now_ms - s.first_ms < SESSION_MAX_AGE_MS);

    out
}

/// Spawn the delay collector (DB-gated). Reuses OIS's shared feed snapshot; DB writes happen off the
/// feed lock. Dedupes on the snapshot's source timestamp so a re-served snapshot is skipped.
pub fn spawn_collector(pool: PgPool, feed: FeedState, runways: Arc<RunwayDb>) {
    tokio::spawn(async move {
        let mut state = DelayState::default();
        let mut last_source = String::new();
        let mut ticker = tokio::time::interval(Duration::from_secs(COLLECT_SECS));
        loop {
            ticker.tick().await;
            let (snap, airports) = {
                let guard = feed.read().await;
                match guard.snapshot.clone() {
                    Some(s) => (s, guard.airports.clone()),
                    None => continue,
                }
            };
            if snap.source_timestamp.is_empty() || snap.source_timestamp == last_source {
                continue;
            }
            last_source = snap.source_timestamp.clone();
            let now = chrono::DateTime::parse_from_rfc3339(&snap.source_timestamp)
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or(snap.fetched_at);

            let legs = process(&mut state, &airports, &runways, &snap.data, now);
            if legs.is_empty() {
                continue;
            }
            match repo::insert_flight_legs(&pool, &legs).await {
                Ok(()) => tracing::info!(count = legs.len(), "delays: recorded flight legs"),
                Err(_) => tracing::warn!("delays: leg insert failed"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::vatsim::{FlightPlan, Pilot, VatsimData};

    fn t(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    fn one(lat: f64, gs: i64, alt: i64, dep: &str, arr: &str, route: &str) -> VatsimData {
        VatsimData {
            pilots: vec![Pilot {
                callsign: "AAL1".into(),
                latitude: lat,
                longitude: -74.0,
                altitude: alt,
                groundspeed: gs,
                heading: 0,
                flight_plan: Some(FlightPlan {
                    departure: dep.into(),
                    arrival: arr.into(),
                    route: route.into(),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    // KAAA at (40.0, -74.0); ~60 nm of latitude ≈ 1°, so lat 40.0 + n/60 is ~n nm north.
    fn airports() -> AirportDb {
        HashMap::from([("KAAA".to_string(), Airport::at(40.0, -74.0))])
    }

    #[test]
    fn records_a_departure_taxi_out() {
        let (ap, rw) = (airports(), RunwayDb::default());
        let mut st = DelayState::default();
        // At the field: watching → rolling (t=30) → airborne (t=120).
        process(
            &mut st,
            &ap,
            &rw,
            &one(40.0, 0, 0, "KAAA", "KBBB", "GLASR3 SEA"),
            t(0),
        );
        process(
            &mut st,
            &ap,
            &rw,
            &one(40.0, 20, 0, "KAAA", "KBBB", "GLASR3 SEA"),
            t(30),
        );
        let legs = process(
            &mut st,
            &ap,
            &rw,
            &one(40.0, 80, 400, "KAAA", "KBBB", "GLASR3 SEA"),
            t(120),
        );
        assert_eq!(legs.len(), 1);
        assert_eq!(legs[0].kind, "departure");
        assert_eq!(legs[0].airport, "KAAA");
        assert_eq!(legs[0].duration_sec, 90); // rolled at 30, wheels-up at 120
        assert_eq!(legs[0].procedure.as_deref(), Some("GLASR")); // SID base from the front
        assert!(st.dep.is_empty());
    }

    #[test]
    fn records_an_arrival_transit_from_the_entry_ring() {
        let (ap, rw) = (airports(), RunwayDb::default());
        let mut st = DelayState::default();
        // ~50 nm out (tracking begins) → ~30 nm (crosses the 40 nm ring, t=600) → touchdown (t=1800).
        process(
            &mut st,
            &ap,
            &rw,
            &one(40.833, 300, 10_000, "KZZZ", "KAAA", "PARCH3"),
            t(0),
        );
        process(
            &mut st,
            &ap,
            &rw,
            &one(40.5, 300, 6_000, "KZZZ", "KAAA", "PARCH3"),
            t(600),
        );
        let legs = process(
            &mut st,
            &ap,
            &rw,
            &one(40.0, 20, 0, "KZZZ", "KAAA", "PARCH3"),
            t(1800),
        );
        assert_eq!(legs.len(), 1);
        assert_eq!(legs[0].kind, "arrival");
        assert_eq!(legs[0].airport, "KAAA");
        assert_eq!(legs[0].duration_sec, 1200); // entry at 600, touchdown at 1800
        assert!(st.arr.is_empty());
    }

    #[test]
    fn no_arrival_leg_when_first_seen_inside_the_ring() {
        let (ap, rw) = (airports(), RunwayDb::default());
        let mut st = DelayState::default();
        // First seen at ~30 nm (already inside the ring) → never tracked → no entry timing.
        process(
            &mut st,
            &ap,
            &rw,
            &one(40.5, 300, 6_000, "KZZZ", "KAAA", ""),
            t(0),
        );
        let legs = process(
            &mut st,
            &ap,
            &rw,
            &one(40.0, 20, 0, "KZZZ", "KAAA", ""),
            t(600),
        );
        assert!(legs.is_empty());
    }
}
