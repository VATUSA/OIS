//! Pushback, start-up, and taxi-out observation collector. A background worker reads OIS's shared
//! feed snapshot each cycle and records, per completed departure, into `stats.taxi_observation`
//! (#277):
//!   * **pushback** — the first movement burst from the gate: push start → push stop.
//!   * **start-up** — the stationary gap after the push (engine start, tug disconnect): push stop →
//!     taxi start.
//!   * **taxi-out** — the next movement burst: taxi start → wheels-up (>60 kt or a >100 ft climb).
//!
//! Movement is the aircraft's *position* changing between updates, at any groundspeed — a tug
//! pushes at 1–3 kt, far below any speed threshold. A phase change is confirmed only after
//! [`CONFIRM_UPDATES`] consecutive updates contradict the current phase (plus [`BURST_MIN_M`] of
//! accumulated travel for a movement burst, rejecting position jitter). At the ~15 s feed cadence a
//! boundary resolves to the update it was first seen on; a push and start-up that both complete
//! inside one poll can collapse into the taxi figure (the estimator's defaults cover that).
//!
//! A push is the first burst while it stays below [`GS_START`] and within [`PUSH_MAX_M`] of where
//! the aircraft was parked. A burst that breaks either limit is taxi — a powerback, a no-tug
//! gate-out, or a slow GA taxi — unless it paused first: then the push ended at that pause and the
//! taxi started where movement resumed (a short start-up the stop confirmation couldn't catch).
//! Without a pause, pushback and start-up are `None`. Once taxiing, stops (hold short, queues)
//! stay taxi.
//!
//! Departure filtering (proximity, arriving turnarounds, airborne thresholds) mirrors
//! `feed/delays.rs`'s departure half; this module additionally matches the spawn point to the
//! nearest defined gate (`flow.airport_gate`, #164 sub-issue A) and tags the departure runway
//! (reusing `delays::nearest_runway`). The raw observations feed the per-gate/type/runway estimator
//! (`feed::taxi_estimate`).
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

// `feed::flow::TAXI_ROLL_GS_KT` (#164 sub-issue E): the speed a tug push never reaches, so a burst
// that does is taxi. One shared constant, not two.
use super::flow::TAXI_ROLL_GS_KT as GS_START;
const GS_STOP: i64 = 60; // kt — airborne
const ALT_CLIMB_FT: i64 = 100;
const DEP_PROX_NM: f64 = 15.0;
const MIN_TAXI_SEC: i64 = 3;
const MAX_TAXI_SEC: i64 = 60 * 60;
const SESSION_MAX_AGE_MS: i64 = 3 * 60 * 60_000;
const COLLECT_SECS: u64 = 15;
/// Position change between two updates that counts as "moving" (below it: jitter/stationary).
const MOVE_M: f64 = 2.0;
/// Accumulated travel a movement burst needs before it's confirmed.
const BURST_MIN_M: f64 = 20.0;
/// Consecutive contradicting updates that confirm a phase change (">2 updates").
const CONFIRM_UPDATES: u32 = 3;
/// Farthest a pushback moves from where the aircraft was parked; beyond it the burst is taxi.
const PUSH_MAX_M: f64 = 150.0;
const M_PER_NM: f64 = 1852.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    Parked,
    PushingBack,
    StartUp,
    Taxiing,
}

impl Phase {
    fn is_moving(self) -> bool {
        matches!(self, Phase::PushingBack | Phase::Taxiing)
    }
}

/// Consecutive updates contradicting the current phase — a candidate phase change, started at
/// `start_ms` (which becomes the phase boundary once confirmed).
#[derive(Clone, Copy)]
struct Run {
    updates: u32,
    start_ms: i64,
    dist_m: f64,
    peak_gs: i64,
}

struct Session {
    dep: String,
    phase: Phase,
    first_seen_ms: i64,
    first_lat: f64,
    first_lon: f64,
    last_lat: f64,
    last_lon: f64,
    run: Option<Run>,
    push_start_ms: Option<i64>,
    push_stop_ms: Option<i64>,
    taxi_start_ms: Option<i64>,
    /// `(stop start, resume)` of the latest brief stop inside a push — too short to confirm as the
    /// push stop, but where the push ended if the burst turns out to continue as taxi.
    last_pause: Option<(i64, i64)>,
    base_alt: i64,
}

impl Session {
    /// Feed one on-ground update into the phase machine.
    fn advance(&mut self, now_ms: i64, lat: f64, lon: f64, gs: i64, alt: i64) {
        let step_m = gc_dist(self.last_lat, self.last_lon, lat, lon) * M_PER_NM;
        (self.last_lat, self.last_lon) = (lat, lon);
        let from_stand_m = gc_dist(self.first_lat, self.first_lon, lat, lon) * M_PER_NM;

        // A "push" that reaches taxi speed or travels too far is taxiing: it split at its last pause
        // (a stop still pending now, or an earlier brief one), else the whole burst was the taxi.
        if self.phase == Phase::PushingBack && (gs > GS_START || from_stand_m > PUSH_MAX_M) {
            let pause = self.run.map(|r| (r.start_ms, now_ms)).or(self.last_pause);
            match pause {
                Some((stop, resume)) => {
                    self.push_stop_ms = Some(stop);
                    self.taxi_start_ms = Some(resume);
                }
                None => self.taxi_start_ms = self.push_start_ms.take(),
            }
            self.start_taxi(alt);
            return;
        }
        if self.phase == Phase::Taxiing {
            return;
        }

        let moving = step_m >= MOVE_M;
        if moving == self.phase.is_moving() {
            if let (Phase::PushingBack, Some(stop)) = (self.phase, self.run) {
                self.last_pause = Some((stop.start_ms, now_ms));
            }
            self.run = None;
            return;
        }
        let run = self.run.get_or_insert(Run {
            updates: 0,
            start_ms: now_ms,
            dist_m: 0.0,
            peak_gs: 0,
        });
        run.updates += 1;
        run.dist_m += step_m;
        run.peak_gs = run.peak_gs.max(gs);
        if run.updates < CONFIRM_UPDATES || (moving && run.dist_m < BURST_MIN_M) {
            return;
        }

        let run = *run;
        self.run = None;
        match self.phase {
            Phase::Parked if run.peak_gs <= GS_START && from_stand_m <= PUSH_MAX_M => {
                self.phase = Phase::PushingBack;
                self.push_start_ms = Some(run.start_ms);
            }
            Phase::Parked | Phase::StartUp => {
                self.taxi_start_ms = Some(run.start_ms);
                self.start_taxi(alt);
            }
            Phase::PushingBack => {
                self.phase = Phase::StartUp;
                self.push_stop_ms = Some(run.start_ms);
            }
            Phase::Taxiing => unreachable!("returned above"),
        }
    }

    fn start_taxi(&mut self, alt: i64) {
        self.phase = Phase::Taxiing;
        self.run = None;
        self.last_pause = None;
        self.base_alt = alt;
    }

    /// `(pushback_sec, startup_sec, taxi_sec)` for a departure that just went airborne at `now_ms`.
    /// Taxi starts at the confirmed taxi burst — or, for a departure quicker than the confirmation
    /// window, at a burst still being confirmed, the push stop, or first-seen, in that order.
    fn timings(&self, now_ms: i64) -> (Option<i32>, Option<i32>, i64) {
        let taxi_start = match self.phase {
            Phase::Taxiing => self.taxi_start_ms,
            Phase::PushingBack => self.push_start_ms, // never stopped: it was the taxi
            Phase::Parked | Phase::StartUp => self.run.map(|r| r.start_ms),
        }
        .or(self.push_stop_ms)
        .unwrap_or(self.first_seen_ms);
        let secs = |from: i64, to: i64| ((to - from) / 1000) as i32;
        let pushback_sec = self
            .push_start_ms
            .zip(self.push_stop_ms)
            .map(|(a, b)| secs(a, b));
        let startup_sec = self.push_stop_ms.map(|stop| secs(stop, taxi_start));
        (pushback_sec, startup_sec, (now_ms - taxi_start) / 1000)
    }
}

#[derive(Default)]
pub struct TaxiObsState {
    dep: HashMap<String, Session>,
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
    startup_sec: Option<i32>,
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
            // A climb only counts once taxiing: a parked aircraft's altitude can drift.
            let airborne =
                gs > GS_STOP || (s.phase == Phase::Taxiing && alt >= s.base_alt + ALT_CLIMB_FT);
            if !airborne {
                s.advance(now_ms, p.latitude, p.longitude, gs, alt);
                continue;
            }
            let (pushback_sec, startup_sec, dur) = s.timings(now_ms);
            if (MIN_TAXI_SEC..=MAX_TAXI_SEC).contains(&dur) {
                out.push(RawObservation {
                    airport: dep.clone(),
                    lat: s.first_lat,
                    lon: s.first_lon,
                    aircraft: (!fp.aircraft_short.is_empty()).then(|| fp.aircraft_short.clone()),
                    runway: nearest_runway(runways, &dep, p.heading),
                    pushback_sec,
                    startup_sec,
                    taxi_sec: dur as i32,
                    observed_at: now,
                });
            }
            done.push(p.callsign.clone());
        } else if let Some(&(dlat, dlon)) = airports.get(&dep) {
            // Exclude pattern work / touch-and-goes / short dep-arr hops: sitting at the field at
            // low speed while also near this same flight plan's arrival airport means we're
            // watching an arrival taxi-in (or a circuit), not a genuine pushback — matching
            // feed/delays.rs's departure half.
            let arriving_turnaround = gs <= GS_STOP
                && airports.get(&arr).is_some_and(|&(alat, alon)| {
                    gc_dist(p.latitude, p.longitude, alat, alon) < 5.0
                });
            // Only start watching an aircraft first seen below taxi speed. One already moving (a
            // backend restart mid-taxi, or the tick right after a recorded departure while still
            // near the field) has no knowable taxi start — recording it produced short, duplicate
            // taxi figures that skew the learned medians.
            if !arriving_turnaround
                && gs <= GS_START
                && gc_dist(p.latitude, p.longitude, dlat, dlon) <= DEP_PROX_NM
            {
                state.dep.insert(
                    p.callsign.clone(),
                    Session {
                        dep: dep.clone(),
                        phase: Phase::Parked,
                        first_seen_ms: now_ms,
                        first_lat: p.latitude,
                        first_lon: p.longitude,
                        last_lat: p.latitude,
                        last_lon: p.longitude,
                        run: None,
                        push_start_ms: None,
                        push_stop_ms: None,
                        taxi_start_ms: None,
                        last_pause: None,
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
                    gate_id: super::flow::nearest_gate(gates_here, r.lat, r.lon),
                    aircraft: r.aircraft,
                    runway: r.runway,
                    pushback_sec: r.pushback_sec,
                    startup_sec: r.startup_sec,
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

    /// Latitude `m` metres north of the KAAA reference point.
    fn north(m: f64) -> f64 {
        40.0 + m / 111_195.0
    }

    /// Feed `(t, metres north, gs, alt)` updates in order; returns whatever the last tick recorded.
    fn run_track(st: &mut TaxiObsState, track: &[(i64, f64, i64, i64)]) -> Vec<RawObservation> {
        let (ap, rw) = (airports(), RunwayDb::default());
        let mut obs = Vec::new();
        for &(secs, m, gs, alt) in track {
            obs = process(st, &ap, &rw, &one(north(m), gs, alt, "KAAA"), t(secs));
        }
        obs
    }

    #[test]
    fn records_separate_pushback_startup_and_taxi_phases() {
        let mut st = TaxiObsState::default();
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0),
                // A 2 kt tug push — never crosses the 7 kt taxi threshold. Onset at t=15, confirmed
                // at t=45 once three moving updates have covered 20 m.
                (15, 7.0, 2, 0),
                (30, 14.0, 2, 0),
                (45, 21.0, 2, 0),
                (60, 28.0, 2, 0),
                // Push stops at t=75 (confirmed after 3 still updates), then start-up to t=180.
                (75, 28.0, 0, 0),
                (90, 28.5, 0, 0),
                (105, 28.5, 0, 0),
                (150, 28.5, 0, 0),
                (180, 28.5, 0, 0),
                // Taxi burst from t=195.
                (195, 60.0, 10, 0),
                (210, 120.0, 15, 0),
                (225, 180.0, 15, 0),
                // Hold short — still taxi.
                (240, 180.0, 0, 0),
                (285, 180.0, 0, 0),
                (300, 180.0, 0, 0),
                // Wheels-up.
                (345, 900.0, 80, 400),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].airport, "KAAA");
        assert_eq!(obs[0].aircraft.as_deref(), Some("B738"));
        assert_eq!(obs[0].pushback_sec, Some(60)); // 15 → 75, not the 75 s of gate dwell before it
        assert_eq!(obs[0].startup_sec, Some(120)); // 75 → 195
        assert_eq!(obs[0].taxi_sec, 150); // 195 → 345
        assert!(st.dep.is_empty());
    }

    #[test]
    fn position_jitter_and_short_shuffles_do_not_start_a_burst() {
        let mut st = TaxiObsState::default();
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0),
                // Sub-2 m jitter, many updates.
                (15, 1.0, 0, 0),
                (30, 0.0, 0, 0),
                (45, 1.5, 0, 0),
                (60, 0.5, 0, 0),
                // Three moving updates, but only 9 m in total — then still again.
                (75, 3.5, 1, 0),
                (90, 6.5, 1, 0),
                (105, 9.5, 1, 0),
                (120, 9.5, 0, 0),
                // The real departure: straight to taxi, then airborne.
                (135, 60.0, 12, 0),
                (150, 130.0, 15, 0),
                (165, 200.0, 15, 0),
                (285, 900.0, 80, 400),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, None);
        assert_eq!(obs[0].startup_sec, None);
        assert_eq!(obs[0].taxi_sec, 150); // 135 → 285
    }

    #[test]
    fn a_no_tug_departure_that_stops_at_hold_short_has_no_pushback_or_startup() {
        let mut st = TaxiObsState::default();
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0),
                // Powerback/gate-out at taxi speed — the first burst is the taxi.
                (15, 30.0, 8, 0),
                (30, 90.0, 12, 0),
                (45, 150.0, 12, 0),
                // A long hold-short stop must not read as push stop + start-up.
                (60, 150.0, 0, 0),
                (75, 150.0, 0, 0),
                (90, 150.0, 0, 0),
                (120, 150.0, 0, 0),
                (135, 900.0, 80, 400),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, None);
        assert_eq!(obs[0].startup_sec, None);
        assert_eq!(obs[0].taxi_sec, 120); // 15 → 135
    }

    #[test]
    fn a_slow_burst_that_reaches_taxi_speed_is_reclassified_as_taxi() {
        let mut st = TaxiObsState::default();
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0),
                // Confirmed as a push (peak 6 kt) at t=45 ...
                (15, 5.0, 3, 0),
                (30, 12.0, 5, 0),
                (45, 22.0, 6, 0),
                // ... but it keeps accelerating past 7 kt without stopping: it was the taxi.
                (60, 60.0, 12, 0),
                // A hold-short stop long enough to confirm a push stop, had the burst stayed a push.
                (75, 60.0, 0, 0),
                (90, 60.0, 0, 0),
                (105, 60.0, 0, 0),
                (180, 900.0, 80, 400),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, None);
        assert_eq!(obs[0].startup_sec, None);
        assert_eq!(obs[0].taxi_sec, 165); // 15 → 180
    }

    #[test]
    fn a_short_startup_before_taxi_speed_splits_the_push_at_its_pause() {
        let mut st = TaxiObsState::default();
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0),
                // Tug push t=15..75 (confirmed at t=45).
                (15, 7.0, 2, 0),
                (30, 14.0, 2, 0),
                (45, 21.0, 2, 0),
                (60, 28.0, 2, 0),
                // Only two still updates — too short to confirm the push stop ...
                (75, 28.0, 0, 0),
                (90, 28.0, 0, 0),
                // ... then off at taxi speed.
                (105, 60.0, 12, 0),
                (120, 150.0, 15, 0),
                (300, 900.0, 80, 400),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, Some(60)); // 15 → 75
        assert_eq!(obs[0].startup_sec, Some(30)); // 75 → 105
        assert_eq!(obs[0].taxi_sec, 195); // 105 → 300
    }

    #[test]
    fn a_short_startup_then_a_slow_taxi_splits_the_push_at_its_pause() {
        let mut st = TaxiObsState::default();
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0),
                (15, 7.0, 2, 0),
                (30, 14.0, 2, 0),
                (45, 21.0, 2, 0),
                (60, 28.0, 2, 0),
                (75, 28.0, 0, 0),
                (90, 28.0, 0, 0),
                // A 4 kt taxi — never taxi speed, but it leaves the stand's push radius.
                (105, 60.0, 4, 0),
                (120, 90.0, 4, 0),
                (135, 120.0, 4, 0),
                (150, 155.0, 4, 0),
                (300, 900.0, 80, 400),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, Some(60));
        assert_eq!(obs[0].startup_sec, Some(30));
        assert_eq!(obs[0].taxi_sec, 195);
    }

    #[test]
    fn a_slow_ga_taxi_with_a_run_up_is_not_a_push() {
        let mut st = TaxiObsState::default();
        // 5 kt (~39 m per update) for 4 minutes, a 90 s run-up, then departure.
        let mut track: Vec<(i64, f64, i64, i64)> = vec![(0, 0.0, 0, 0)];
        for i in 1..=16 {
            track.push((i * 15, i as f64 * 39.0, 5, 0));
        }
        for i in 1..=6 {
            track.push((240 + i * 15, 624.0, 0, 0));
        }
        track.push((345, 1500.0, 80, 400));

        let obs = run_track(&mut st, &track);

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, None);
        assert_eq!(obs[0].startup_sec, None);
        assert_eq!(obs[0].taxi_sec, 330); // 15 → 345, run-up included
    }

    #[test]
    fn a_burst_already_past_the_push_radius_when_confirmed_is_taxi() {
        let mut st = TaxiObsState::default();
        // 7 kt — at or below GS_START, but 54 m per update, so the three confirming updates already
        // cover more than PUSH_MAX_M. The hold-short stop would otherwise confirm a push stop.
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0),
                (15, 54.0, 7, 0),
                (30, 108.0, 7, 0),
                (45, 162.0, 7, 0),
                (60, 162.0, 0, 0),
                (75, 162.0, 0, 0),
                (90, 162.0, 0, 0),
                (105, 162.0, 0, 0),
                (120, 400.0, 30, 0),
                (135, 1200.0, 90, 300),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, None);
        assert_eq!(obs[0].startup_sec, None);
        assert_eq!(obs[0].taxi_sec, 120); // 15 → 135
    }

    #[test]
    fn airborne_straight_out_of_a_push_times_taxi_from_the_push_start() {
        let mut st = TaxiObsState::default();
        // Confirmed as a push at t=45, then past GS_STOP on the very next update: the burst was the
        // taxi, timed from where it began — not from first-seen, which would add the gate dwell.
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0),
                (15, 7.0, 2, 0),
                (30, 14.0, 2, 0),
                (45, 21.0, 2, 0),
                (60, 300.0, 65, 0),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, None);
        assert_eq!(obs[0].startup_sec, None);
        assert_eq!(obs[0].taxi_sec, 45); // 15 → 60, not 60 from first-seen
    }

    #[test]
    fn a_two_update_shuffle_is_not_a_confirmed_push() {
        let mut st = TaxiObsState::default();
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0),
                // Two moving updates covering 30 m — one short of confirmation.
                (15, 15.0, 2, 0),
                (30, 30.0, 2, 0),
                (45, 30.0, 0, 0),
                (60, 30.0, 0, 0),
                (75, 30.0, 0, 0),
                (90, 30.0, 0, 0),
                (105, 80.0, 12, 0),
                (120, 150.0, 15, 0),
                (135, 220.0, 15, 0),
                (255, 900.0, 80, 400),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, None);
        assert_eq!(obs[0].taxi_sec, 150); // 105 → 255
    }

    #[test]
    fn a_parked_aircraft_whose_altitude_drifts_is_not_recorded() {
        let mut st = TaxiObsState::default();
        let obs = run_track(
            &mut st,
            &[(0, 0.0, 0, 0), (15, 0.0, 0, 150), (30, 0.0, 0, 200)],
        );

        assert!(obs.is_empty());
        assert_eq!(st.dep.len(), 1);
    }

    #[test]
    fn a_departure_airborne_mid_confirmation_times_taxi_from_that_burst() {
        let mut st = TaxiObsState::default();
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0),
                (15, 0.0, 0, 0),
                // Two moving updates — the taxi burst isn't confirmed yet when it lifts off.
                (30, 40.0, 10, 0),
                (45, 120.0, 20, 0),
                (60, 900.0, 80, 400),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].taxi_sec, 30); // 30 → 60, not first-seen (60 s)
    }

    #[test]
    fn a_climb_is_measured_from_the_altitude_where_taxi_started() {
        let mut st = TaxiObsState::default();
        let taxiing = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0),
                // The taxi burst reports 150 ft above the first-seen altitude (sloped field).
                (15, 60.0, 12, 150),
                (30, 130.0, 15, 150),
                (45, 200.0, 15, 150),
                (60, 270.0, 15, 160),
            ],
        );
        assert!(
            taxiing.is_empty(),
            "a 10 ft change while taxiing isn't a takeoff"
        );

        let obs = run_track(&mut st, &[(180, 900.0, 80, 600)]);
        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].taxi_sec, 165); // 15 → 180
    }

    #[test]
    fn an_aircraft_first_seen_already_moving_is_not_recorded() {
        let (ap, rw) = (airports(), RunwayDb::default());
        let mut st = TaxiObsState::default();
        // First tick already shows it rolling — its taxi start is unknown, so no observation.
        process(&mut st, &ap, &rw, &one(40.0, 20, 0, "KAAA"), t(0));
        let obs = process(&mut st, &ap, &rw, &one(40.0, 80, 400, "KAAA"), t(60));
        assert!(obs.is_empty());
        assert!(st.dep.is_empty());
    }

    #[test]
    fn a_recorded_departure_is_not_recorded_again_while_still_near_the_field() {
        let (ap, rw) = (airports(), RunwayDb::default());
        let mut st = TaxiObsState::default();
        process(&mut st, &ap, &rw, &one(40.0, 0, 0, "KAAA"), t(0));
        let first = process(&mut st, &ap, &rw, &one(40.0, 80, 100, "KAAA"), t(50));
        assert_eq!(first.len(), 1);
        // Next ticks: fast but still low (<500 ft) and inside the departure proximity radius.
        let again = process(&mut st, &ap, &rw, &one(40.01, 140, 300, "KAAA"), t(65));
        let again2 = process(&mut st, &ap, &rw, &one(40.03, 160, 450, "KAAA"), t(80));
        assert!(again.is_empty() && again2.is_empty());
        assert!(st.dep.is_empty());
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
        // Below GS_START, so it is the turnaround filter doing the rejecting and not the
        // "first seen already moving" gate — at taxi speed this would pass either way.
        process(
            &mut st,
            &ap,
            &rw,
            &one_with_arrival(40.0, 3, 0, "KAAA", "KBBB"),
            t(0),
        );

        assert!(st.dep.is_empty());
    }
}
