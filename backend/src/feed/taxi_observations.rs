//! Pushback+startup and taxi-out observation collector. A background worker reads OIS's shared
//! feed snapshot each cycle and records, per completed departure, into `stats.taxi_observation`:
//!   * **pushback+startup** — first-seen (parked) → start-of-taxi (~7 kt).
//!   * **taxi-out**         — start-of-taxi (~7 kt) → wheels-up (>60 kt or a >100 ft climb).
//!
//! Departure detection mirrors `feed/delays.rs`'s departure half exactly (same thresholds); this
//! module additionally matches the spawn point to the nearest defined gate (`flow.airport_gate`,
//! #164 sub-issue A) and tags the departure runway (reusing `delays::nearest_runway`). The raw
//! observations feed a later per-gate/type/runway estimator (#164 sub-issue D).
//!
//! `process()` stays pure and DB-free (like `delays::process`) so it's directly unit-testable;
//! gate resolution reads `AppState::gates` (kept current by `jobs::spawn_airport_gates_refresh`
//! and force-reloaded on write by `handlers::airport_surface`) — the feed subsystem itself never
//! queries the DB inline, matching the rest of `feed/*`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use chrono::{DateTime, Utc};
use sqlx::PgPool;

use super::FeedState;
use super::airports::AirportDb;
use super::delays::nearest_runway;
use super::flow::gc_dist;
use super::runway_db::RunwayDb;
use super::vatsim::VatsimData;
use crate::models::AirportGateBody;
use crate::repos::stats::{self as repo, TaxiObservationRow};

// Matches feed/taxi.rs / feed/delays.rs's own departure-taxi boundary.
const GS_START: i64 = 7; // kt — taxi has begun
const GS_STOP: i64 = 60; // kt — airborne
const ALT_CLIMB_FT: i64 = 100;
const DEP_PROX_NM: f64 = 15.0;
const MIN_TAXI_SEC: i64 = 3;
const MAX_TAXI_SEC: i64 = 60 * 60;
const SESSION_MAX_AGE_MS: i64 = 3 * 60 * 60_000;
const COLLECT_SECS: u64 = 15;

/// A gate/parking spot must be within this of the spawn point to count as a match.
const GATE_MATCH_MAX_NM: f64 = 0.06; // ~360 ft

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Watching,
    Rolling,
}

struct Session {
    dep: String,
    phase: Phase,
    first_seen_ms: i64,
    first_lat: f64,
    first_lon: f64,
    start_ms: Option<i64>,
    base_alt: i64,
}

#[derive(Default)]
pub struct TaxiObsState {
    dep: HashMap<String, Session>,
}

/// The gate at `icao` nearest `(lat, lon)`, within `GATE_MATCH_MAX_NM`, else `None`.
fn nearest_gate(gates: &[AirportGateBody], lat: f64, lon: f64) -> Option<String> {
    let mut best: Option<(String, f64)> = None;
    for g in gates {
        let d = gc_dist(lat, lon, g.lat, g.lon);
        if best.as_ref().is_none_or(|(_, bd)| d < *bd) {
            best = Some((g.id.clone(), d));
        }
    }
    best.filter(|(_, d)| *d <= GATE_MATCH_MAX_NM)
        .map(|(id, _)| id)
}

/// One completed departure's timings, with `gate_id` left unresolved (`spawn_collector` fills it
/// in once it has the airport's gates — see the module doc).
struct RawObservation {
    airport: String,
    lat: f64,
    lon: f64,
    aircraft: Option<String>,
    runway: Option<String>,
    pushback_sec: Option<i32>,
    taxi_sec: i32,
    observed_at: DateTime<Utc>,
}

fn process(
    state: &mut TaxiObsState,
    airports: &AirportDb,
    runways: &RunwayDb,
    data: &VatsimData,
    now: DateTime<Utc>,
) -> Vec<RawObservation> {
    let now_ms = now.timestamp_millis();
    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut done: Vec<String> = Vec::new();

    for p in &data.pilots {
        let Some(fp) = &p.flight_plan else { continue };
        let gs = p.groundspeed;
        let alt = p.altitude;
        let dep = fp.departure.to_ascii_uppercase();
        let arr = fp.arrival.to_ascii_uppercase();
        if dep.is_empty() {
            continue;
        }

        if let Some(s) = state.dep.get_mut(&p.callsign).filter(|s| s.dep == dep) {
            seen.insert(p.callsign.clone());
            if s.phase == Phase::Watching && gs > GS_START {
                s.phase = Phase::Rolling;
                s.start_ms = Some(now_ms);
                s.base_alt = alt;
            }
            if s.phase == Phase::Rolling
                && (gs > GS_STOP || alt >= s.base_alt + ALT_CLIMB_FT)
                && let Some(start) = s.start_ms
            {
                // A very short observed roll means we caught it mid-taxi; fall back to
                // first-seen, matching feed/delays.rs's departure half.
                let start_ms = if now_ms - start < 3_000 && s.first_seen_ms < start {
                    s.first_seen_ms
                } else {
                    start
                };
                let dur = (now_ms - start_ms) / 1000;
                if (MIN_TAXI_SEC..=MAX_TAXI_SEC).contains(&dur) {
                    let pushback_sec = (start_ms > s.first_seen_ms)
                        .then(|| ((start_ms - s.first_seen_ms) / 1000) as i32);
                    out.push(RawObservation {
                        airport: dep.clone(),
                        lat: s.first_lat,
                        lon: s.first_lon,
                        aircraft: (!fp.aircraft_short.is_empty())
                            .then(|| fp.aircraft_short.clone()),
                        runway: nearest_runway(runways, &dep, p.heading),
                        pushback_sec,
                        taxi_sec: dur as i32,
                        observed_at: now,
                    });
                }
                done.push(p.callsign.clone());
            }
        } else if let Some(&(dlat, dlon)) = airports.get(&dep) {
            // Exclude pattern work / touch-and-goes / short dep-arr hops: sitting at the field at
            // low speed while also near this same flight plan's arrival airport means we're
            // watching an arrival taxi-in (or a circuit), not a genuine pushback — matching
            // feed/delays.rs's departure half.
            let arriving_turnaround = gs <= GS_STOP
                && airports.get(&arr).is_some_and(|&(alat, alon)| {
                    gc_dist(p.latitude, p.longitude, alat, alon) < 5.0
                });
            if !arriving_turnaround
                && gc_dist(p.latitude, p.longitude, dlat, dlon) <= DEP_PROX_NM
                && !(gs > GS_STOP && alt > 500)
            {
                // If we're already past the roll threshold the very first time we see this
                // departure, pushback happened before we started watching — record it as
                // already-Rolling with no measurable pushback (start_ms == first_seen_ms), rather
                // than always starting Watching and only discovering "rolling" a tick later.
                let already_rolling = gs > GS_START;
                state.dep.insert(
                    p.callsign.clone(),
                    Session {
                        dep: dep.clone(),
                        phase: if already_rolling {
                            Phase::Rolling
                        } else {
                            Phase::Watching
                        },
                        first_seen_ms: now_ms,
                        first_lat: p.latitude,
                        first_lon: p.longitude,
                        start_ms: already_rolling.then_some(now_ms),
                        base_alt: alt,
                    },
                );
                seen.insert(p.callsign.clone());
            }
        }
    }

    for cs in done {
        state.dep.remove(&cs);
    }
    state
        .dep
        .retain(|cs, s| seen.contains(cs) && now_ms - s.first_seen_ms < SESSION_MAX_AGE_MS);

    out
}

/// Spawn the taxi-observation collector (DB-gated). Reuses OIS's shared feed snapshot, exactly
/// like `feed::delays::spawn_collector`; dedupes on the snapshot's source timestamp. Gate matching
/// reads `gates` (kept current by `jobs::spawn_airport_gates_refresh`) — no DB read in the loop.
pub fn spawn_collector(
    pool: PgPool,
    feed: FeedState,
    runways: Arc<RunwayDb>,
    gates: Arc<ArcSwap<HashMap<String, Vec<AirportGateBody>>>>,
) {
    tokio::spawn(async move {
        let mut state = TaxiObsState::default();
        let mut last_source = String::new();
        let empty_gates: Vec<AirportGateBody> = Vec::new();
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
            let Ok(now) = chrono::DateTime::parse_from_rfc3339(&snap.source_timestamp) else {
                continue;
            };
            let raw = process(&mut state, &airports, &runways, &snap.data, now.into());
            if raw.is_empty() {
                continue;
            }

            let by_icao = gates.load();
            let mut rows = Vec::with_capacity(raw.len());
            for r in raw {
                let gates_here = by_icao.get(&r.airport).unwrap_or(&empty_gates);
                rows.push(TaxiObservationRow {
                    airport: r.airport,
                    gate_id: nearest_gate(gates_here, r.lat, r.lon),
                    aircraft: r.aircraft,
                    runway: r.runway,
                    pushback_sec: r.pushback_sec,
                    taxi_sec: r.taxi_sec,
                    observed_at: r.observed_at,
                });
            }
            if let Err(e) = repo::insert_taxi_observations(&pool, &rows).await {
                tracing::warn!(error = ?e, "taxi_observations: insert failed");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::vatsim::{FlightPlan, Pilot};

    fn t(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    fn one(lat: f64, gs: i64, alt: i64, dep: &str) -> VatsimData {
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
                    arrival: "KBBB".into(),
                    aircraft_short: "B738".into(),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn airports() -> AirportDb {
        HashMap::from([("KAAA".to_string(), (40.0, -74.0))])
    }

    /// KAAA and KBBB co-located, for the arriving-turnaround test below.
    fn airports_with_arrival() -> AirportDb {
        HashMap::from([
            ("KAAA".to_string(), (40.0, -74.0)),
            ("KBBB".to_string(), (40.0, -74.0)),
        ])
    }

    fn one_with_arrival(lat: f64, gs: i64, alt: i64, dep: &str, arr: &str) -> VatsimData {
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
                    aircraft_short: "B738".into(),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn records_pushback_and_taxi_out() {
        let (ap, rw) = (airports(), RunwayDb::default());
        let mut st = TaxiObsState::default();
        // Parked at t=0, starts rolling at t=45 (pushback+startup = 45s), airborne at t=135
        // (taxi-out = 90s).
        process(&mut st, &ap, &rw, &one(40.0, 0, 0, "KAAA"), t(0));
        process(&mut st, &ap, &rw, &one(40.0, 20, 0, "KAAA"), t(45));
        let obs = process(&mut st, &ap, &rw, &one(40.0, 80, 400, "KAAA"), t(135));

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].airport, "KAAA");
        assert_eq!(obs[0].pushback_sec, Some(45));
        assert_eq!(obs[0].taxi_sec, 90);
        assert_eq!(obs[0].aircraft.as_deref(), Some("B738"));
        assert!(st.dep.is_empty());
    }

    #[test]
    fn no_pushback_figure_when_already_rolling_on_first_seen() {
        let (ap, rw) = (airports(), RunwayDb::default());
        let mut st = TaxiObsState::default();
        // First tick already shows it rolling — pushback start is unknown, not zero.
        process(&mut st, &ap, &rw, &one(40.0, 20, 0, "KAAA"), t(0));
        let obs = process(&mut st, &ap, &rw, &one(40.0, 80, 400, "KAAA"), t(60));

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, None);
        assert_eq!(obs[0].taxi_sec, 60);
    }

    #[test]
    fn very_short_roll_in_one_tick_falls_back_to_first_seen_instead_of_dropping() {
        let (ap, rw) = (airports(), RunwayDb::default());
        let mut st = TaxiObsState::default();
        // Parked at t=0; by t=50 groundspeed has already jumped past both the roll and stop
        // thresholds between two polls (a fast-accelerating GA departure). Without the
        // first-seen fallback this reads as a zero-second roll (start_ms == now_ms) and is
        // silently dropped instead of recovered.
        process(&mut st, &ap, &rw, &one(40.0, 0, 0, "KAAA"), t(0));
        let obs = process(&mut st, &ap, &rw, &one(40.0, 80, 400, "KAAA"), t(50));

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].taxi_sec, 50);
    }

    #[test]
    fn arriving_turnaround_does_not_start_a_departure_session() {
        let (ap, rw) = (airports_with_arrival(), RunwayDb::default());
        let mut st = TaxiObsState::default();
        // Low groundspeed near both this flight plan's departure AND arrival airport (pattern
        // work, a touch-and-go, or a same-field circuit) must not be mistaken for a pushback.
        process(
            &mut st,
            &ap,
            &rw,
            &one_with_arrival(40.0, 20, 0, "KAAA", "KBBB"),
            t(0),
        );

        assert!(st.dep.is_empty());
    }

    fn gate(id: &str, lat: f64, lon: f64) -> AirportGateBody {
        AirportGateBody {
            id: id.to_string(),
            icao: "KAAA".to_string(),
            name: id.to_string(),
            lat,
            lon,
            source: "manual".to_string(),
            updated_at: Utc::now(),
            editable: false,
        }
    }

    #[test]
    fn nearest_gate_matches_the_closest_within_range() {
        let gates = vec![gate("A1", 40.0, -74.0), gate("A2", 40.01, -74.0)];
        assert_eq!(nearest_gate(&gates, 40.0001, -74.0), Some("A1".to_string()));
        assert_eq!(nearest_gate(&gates, 40.0099, -74.0), Some("A2".to_string()));
    }

    #[test]
    fn nearest_gate_none_when_too_far() {
        let gates = vec![gate("A1", 40.0, -74.0)];
        assert_eq!(nearest_gate(&gates, 41.0, -74.0), None);
        assert_eq!(nearest_gate(&[], 40.0, -74.0), None);
    }
}
