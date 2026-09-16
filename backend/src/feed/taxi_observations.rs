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
//! A push is the first burst while it stays below [`GS_START`] and most of its travel runs *against*
//! the reported heading — a tug pushes the aircraft backwards, taxi runs along the nose (#285).
//! Steps across the nose are a pivoting tug and say nothing either way; where nothing votes, the
//! older distance proxy decides: within [`PUSH_MAX_M`] of where the aircraft was parked. A burst
//! that is neither is taxi — a powerback, a no-tug gate-out, or a slow GA taxi.
//!
//! A push ends when the aircraft moves off after a stop, when it runs forwards for
//! [`FLIP_CONFIRM`] updates (the tug pulling it ahead), or when it reaches [`GS_START`] — at the
//! stop where it paused if there was one, otherwise where it started running forwards. A push that
//! outlasts [`PUSH_MAX_SEC`] or outruns [`PUSH_TOW_MAX_M`] was never a push: the heading must be
//! stale or wrong, so the whole burst becomes taxi. Only a burst that never stopped at all leaves
//! pushback and start-up `None`. Once taxiing, stops (hold short, queues) stay taxi.
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
use super::fca::bearing_deg;
use super::flow::gc_dist;
use super::runway_db::RunwayDb;
use super::taxi_estimate::{PUSHBACK_BOUNDS_SEC, STARTUP_BOUNDS_SEC};
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
/// Farthest a pushback moves from where the aircraft was parked; beyond it the burst is taxi. Only
/// consulted when the heading can't classify the burst (see [`Session::advance`]).
const PUSH_MAX_M: f64 = 150.0;
/// How a step's track relates to the nose. Beyond [`BACKWARDS_DEG`] off it is a tug pushing the
/// aircraft back; within [`FORWARDS_DEG`] it is taxi. In between — which is where a pivoting tug
/// tracks, roughly perpendicular to the fuselage — the step says nothing and is ignored, so noise
/// in that band neither classifies a burst nor ends a push.
const BACKWARDS_DEG: f64 = 120.0;
const FORWARDS_DEG: f64 = 60.0;
/// Consecutive forward steps that end a push in progress. One is noise — a pivoting tug produces
/// forward-reading steps mid-push; two in a row is the tug pulling the aircraft ahead.
const FLIP_CONFIRM: u32 = 2;
/// Farthest a direction-classified push (a tow) may run, and the longest it may last, before it's
/// taxi after all — a stale or wrong heading must not hold a session in `PushingBack` all day.
const PUSH_TOW_MAX_M: f64 = 1500.0;
const PUSH_MAX_SEC: i64 = 20 * 60;
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
    /// Metres travelled against the reported heading, and along it (#285) — weighted by distance,
    /// so a couple of metres of jitter can't outvote the tug.
    back_m: f64,
    fwd_m: f64,
}

impl Run {
    /// Whether this burst is a push by direction: most of its travel runs against the heading.
    /// `None` when the travel can't say (none counted, or an even split) — the caller falls back to
    /// the distance proxy.
    fn backwards(&self) -> Option<bool> {
        (self.back_m != self.fwd_m).then_some(self.back_m > self.fwd_m)
    }
}

/// The shortest signed turn from `a` to `b`, in (-180, 180].
fn angle_diff(a: f64, b: f64) -> f64 {
    let d = (b - a) % 360.0;
    if d > 180.0 {
        d - 360.0
    } else if d <= -180.0 {
        d + 360.0
    } else {
        d
    }
}

/// How a step ran relative to the nose. A turning tug rotates the nose across the step, so the
/// comparison is deliberately coarse — the [`FORWARDS_DEG`]..[`BACKWARDS_DEG`] band absorbs it.
#[derive(Clone, Copy, PartialEq)]
enum StepDir {
    /// Against the nose: a tug pushing the aircraft back.
    Back,
    /// Along the nose: taxi.
    Fwd,
    /// Across the nose — a pivot, or too ambiguous to call.
    Sideways,
}

/// Which way a step from `from` to `to` ran, relative to `hdg`.
fn step_dir(from: (f64, f64), to: (f64, f64), hdg: f64) -> StepDir {
    let track = bearing_deg([from.0, from.1], [to.0, to.1]);
    let off = angle_diff(hdg, track).abs();
    if off > BACKWARDS_DEG {
        StepDir::Back
    } else if off < FORWARDS_DEG {
        StepDir::Fwd
    } else {
        StepDir::Sideways
    }
}

/// A phase duration worth storing, or `None` when it falls outside the estimator's sanity bounds —
/// a figure that far out is a misread track, not a slow tug.
fn sane_sec(secs: i32, bounds: (f64, f64)) -> Option<i32> {
    (bounds.0..=bounds.1)
        .contains(&(secs as f64))
        .then_some(secs)
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
    /// Whether the push was classified by heading rather than the distance fallback — a tow can then
    /// run past [`PUSH_MAX_M`] and stay a push (#285).
    push_by_direction: bool,
    /// Consecutive forward-running steps while pushing back, and when that streak began: the push
    /// ends there once [`FLIP_CONFIRM`] of them confirm the tug is pulling ahead.
    fwd_streak: u32,
    fwd_streak_ms: i64,
    base_alt: i64,
}

impl Session {
    /// Feed one on-ground update into the phase machine.
    fn advance(&mut self, now_ms: i64, lat: f64, lon: f64, gs: i64, alt: i64, hdg: i64) {
        let step_m = gc_dist(self.last_lat, self.last_lon, lat, lon) * M_PER_NM;
        let moving = step_m >= MOVE_M;
        let dir = moving.then(|| step_dir((self.last_lat, self.last_lon), (lat, lon), hdg as f64));
        (self.last_lat, self.last_lon) = (lat, lon);
        let from_stand_m = gc_dist(self.first_lat, self.first_lon, lat, lon) * M_PER_NM;

        if self.phase == Phase::PushingBack {
            // Moving off after a stop, in any direction but backwards, means the push already ended
            // at that stop and this is the taxi — even when the stop was too short to confirm.
            if let (Some(d), Some(stop)) = (dir, self.run)
                && d != StepDir::Back
            {
                self.push_stop_ms = Some(stop.start_ms);
                self.taxi_start_ms = Some(now_ms);
                self.start_taxi(alt);
                return;
            }
            // A pivoting tug throws the odd forward-reading step, so the push only ends once the
            // aircraft keeps running forwards — and it ends *where that started*, since a push we
            // already confirmed happened whatever follows it.
            if dir == Some(StepDir::Fwd) {
                self.fwd_streak += 1;
                if self.fwd_streak == 1 {
                    self.fwd_streak_ms = now_ms;
                }
                if self.push_by_direction && self.fwd_streak >= FLIP_CONFIRM {
                    // If the aircraft stopped just before running forwards, that stop is the push's
                    // real end and the gap before it moved off is the start-up; otherwise the push
                    // ran straight into the pull and ends where that began.
                    let (stop, resume) = self
                        .last_pause
                        .filter(|&(_, resume)| resume == self.fwd_streak_ms)
                        .unwrap_or((self.fwd_streak_ms, self.fwd_streak_ms));
                    self.push_stop_ms = Some(stop);
                    self.taxi_start_ms = Some(resume);
                    self.start_taxi(alt);
                    return;
                }
            } else if dir == Some(StepDir::Back) {
                self.fwd_streak = 0;
            }

            // Reaching taxi speed, or running past what a push plausibly covers, says the burst was
            // never a push: it splits at its last pause (a stop pending now, or an earlier brief
            // one), else the whole burst was the taxi.
            let overrun = if self.push_by_direction {
                from_stand_m > PUSH_TOW_MAX_M
                    || self
                        .push_start_ms
                        .is_some_and(|start| now_ms - start > PUSH_MAX_SEC * 1000)
            } else {
                from_stand_m > PUSH_MAX_M
            };
            if gs > GS_START || overrun {
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
        }
        if self.phase == Phase::Taxiing {
            return;
        }

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
            back_m: 0.0,
            fwd_m: 0.0,
        });
        run.updates += 1;
        run.dist_m += step_m;
        run.peak_gs = run.peak_gs.max(gs);
        match dir {
            Some(StepDir::Back) => run.back_m += step_m,
            Some(StepDir::Fwd) => run.fwd_m += step_m,
            _ => {}
        }
        if run.updates < CONFIRM_UPDATES || (moving && run.dist_m < BURST_MIN_M) {
            return;
        }

        let run = *run;
        self.run = None;
        match self.phase {
            Phase::Parked
                if run.peak_gs <= GS_START
                    && run.backwards().unwrap_or(from_stand_m <= PUSH_MAX_M) =>
            {
                self.phase = Phase::PushingBack;
                self.push_start_ms = Some(run.start_ms);
                self.push_by_direction = run.backwards().is_some();
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
        self.push_by_direction = false;
        self.fwd_streak = 0;
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
                s.advance(now_ms, p.latitude, p.longitude, gs, alt, p.heading);
                continue;
            }
            let (pushback_sec, startup_sec, dur) = s.timings(now_ms);
            // Bound what a phase may claim, the way taxi_sec is bounded below: the estimator only
            // clamps its median, so an absurd figure would still land in stats.taxi_observation.
            let pushback_sec = pushback_sec.and_then(|v| sane_sec(v, PUSHBACK_BOUNDS_SEC));
            let startup_sec = startup_sec.and_then(|v| sane_sec(v, STARTUP_BOUNDS_SEC));
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
                        push_by_direction: false,
                        fwd_streak: 0,
                        fwd_streak_ms: 0,
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
        one_hdg(lat, gs, alt, dep, 0)
    }

    fn one_hdg(lat: f64, gs: i64, alt: i64, dep: &str, hdg: i64) -> VatsimData {
        VatsimData {
            pilots: vec![Pilot {
                callsign: "AAL1".into(),
                latitude: lat,
                longitude: -74.0,
                altitude: alt,
                groundspeed: gs,
                heading: hdg,
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

    /// Feed `(t, metres north, gs, alt, heading)` updates in order; returns whatever the last tick
    /// recorded. Every track moves north, so heading 180 is moving backwards (a tug push) and
    /// heading 0 is moving forwards (taxi) — see #285.
    fn run_track(
        st: &mut TaxiObsState,
        track: &[(i64, f64, i64, i64, i64)],
    ) -> Vec<RawObservation> {
        let (ap, rw) = (airports(), RunwayDb::default());
        let mut obs = Vec::new();
        for &(secs, m, gs, alt, hdg) in track {
            obs = process(
                st,
                &ap,
                &rw,
                &one_hdg(north(m), gs, alt, "KAAA", hdg),
                t(secs),
            );
        }
        obs
    }

    #[test]
    fn records_separate_pushback_startup_and_taxi_phases() {
        let mut st = TaxiObsState::default();
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0, 0),
                // A 2 kt tug push — never crosses the 7 kt taxi threshold. Onset at t=15, confirmed
                // at t=45 once three moving updates have covered 20 m.
                (15, 7.0, 2, 0, 180),
                (30, 14.0, 2, 0, 180),
                (45, 21.0, 2, 0, 180),
                (60, 28.0, 2, 0, 180),
                // Push stops at t=75 (confirmed after 3 still updates), then start-up to t=180.
                (75, 28.0, 0, 0, 180),
                (90, 28.5, 0, 0, 180),
                (105, 28.5, 0, 0, 180),
                (150, 28.5, 0, 0, 180),
                (180, 28.5, 0, 0, 180),
                // Taxi burst from t=195.
                (195, 60.0, 10, 0, 0),
                (210, 120.0, 15, 0, 0),
                (225, 180.0, 15, 0, 0),
                // Hold short — still taxi.
                (240, 180.0, 0, 0, 0),
                (285, 180.0, 0, 0, 0),
                (300, 180.0, 0, 0, 0),
                // Wheels-up.
                (345, 900.0, 80, 400, 0),
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
                (0, 0.0, 0, 0, 0),
                // Sub-2 m jitter, many updates.
                (15, 1.0, 0, 0, 0),
                (30, 0.0, 0, 0, 0),
                (45, 1.5, 0, 0, 0),
                (60, 0.5, 0, 0, 0),
                // Three moving updates, but only 9 m in total — then still again.
                (75, 3.5, 1, 0, 0),
                (90, 6.5, 1, 0, 0),
                (105, 9.5, 1, 0, 0),
                (120, 9.5, 0, 0, 0),
                // The real departure: straight to taxi, then airborne.
                (135, 60.0, 12, 0, 0),
                (150, 130.0, 15, 0, 0),
                (165, 200.0, 15, 0, 0),
                (285, 900.0, 80, 400, 0),
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
                (0, 0.0, 0, 0, 0),
                // Powerback/gate-out at taxi speed — the first burst is the taxi.
                (15, 30.0, 8, 0, 0),
                (30, 90.0, 12, 0, 0),
                (45, 150.0, 12, 0, 0),
                // A long hold-short stop must not read as push stop + start-up.
                (60, 150.0, 0, 0, 0),
                (75, 150.0, 0, 0, 0),
                (90, 150.0, 0, 0, 0),
                (120, 150.0, 0, 0, 0),
                (135, 900.0, 80, 400, 0),
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
                (0, 0.0, 0, 0, 0),
                // Confirmed as a push (peak 6 kt, moving backwards) at t=45 ...
                (15, 5.0, 3, 0, 180),
                (30, 12.0, 5, 0, 180),
                (45, 22.0, 6, 0, 180),
                // ... but it keeps accelerating past 7 kt without stopping: it was the taxi.
                (60, 60.0, 12, 0, 0),
                // A hold-short stop long enough to confirm a push stop, had the burst stayed a push.
                (75, 60.0, 0, 0, 0),
                (90, 60.0, 0, 0, 0),
                (105, 60.0, 0, 0, 0),
                (180, 900.0, 80, 400, 0),
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
                (0, 0.0, 0, 0, 0),
                // Tug push t=15..75 (confirmed at t=45).
                (15, 7.0, 2, 0, 180),
                (30, 14.0, 2, 0, 180),
                (45, 21.0, 2, 0, 180),
                (60, 28.0, 2, 0, 180),
                // Only two still updates — too short to confirm the push stop ...
                (75, 28.0, 0, 0, 180),
                (90, 28.0, 0, 0, 180),
                // ... then off at taxi speed.
                (105, 60.0, 12, 0, 0),
                (120, 150.0, 15, 0, 0),
                (300, 900.0, 80, 400, 0),
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
                // Parked nose-south, pushed north (backwards), turned during the stop.
                (0, 0.0, 0, 0, 180),
                (15, 7.0, 2, 0, 180),
                (30, 14.0, 2, 0, 180),
                (45, 21.0, 2, 0, 180),
                (60, 28.0, 2, 0, 180),
                (75, 28.0, 0, 0, 180),
                (90, 28.0, 0, 0, 180),
                // A 4 kt taxi — never taxi speed, but it leaves the stand's push radius.
                (105, 60.0, 4, 0, 0),
                (120, 90.0, 4, 0, 0),
                (135, 120.0, 4, 0, 0),
                (150, 155.0, 4, 0, 0),
                (300, 900.0, 80, 400, 0),
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
        let mut track: Vec<(i64, f64, i64, i64, i64)> = vec![(0, 0.0, 0, 0, 0)];
        for i in 1..=16 {
            track.push((i * 15, i as f64 * 39.0, 5, 0, 0)); // forward: a taxi, not a push
        }
        for i in 1..=6 {
            track.push((240 + i * 15, 624.0, 0, 0, 0));
        }
        track.push((345, 1500.0, 80, 400, 0));

        let obs = run_track(&mut st, &track);

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, None);
        assert_eq!(obs[0].startup_sec, None);
        assert_eq!(obs[0].taxi_sec, 330); // 15 → 345, run-up included
    }

    #[test]
    fn a_burst_already_past_the_push_radius_when_confirmed_is_taxi() {
        let mut st = TaxiObsState::default();
        // 7 kt — at or below GS_START, but moving forwards, so it's a taxi however far it goes
        // (#285). The hold-short stop would otherwise confirm a push stop.
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0, 0),
                (15, 54.0, 7, 0, 0),
                (30, 108.0, 7, 0, 0),
                (45, 162.0, 7, 0, 0),
                (60, 162.0, 0, 0, 0),
                (75, 162.0, 0, 0, 0),
                (90, 162.0, 0, 0, 0),
                (105, 162.0, 0, 0, 0),
                (120, 400.0, 30, 0, 0),
                (135, 1200.0, 90, 300, 0),
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
                (0, 0.0, 0, 0, 0),
                (15, 7.0, 2, 0, 180),
                (30, 14.0, 2, 0, 180),
                (45, 21.0, 2, 0, 180),
                (60, 300.0, 65, 0, 0),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, None);
        assert_eq!(obs[0].startup_sec, None);
        assert_eq!(obs[0].taxi_sec, 45); // 15 → 60, not 60 from first-seen
    }

    #[test]
    fn a_no_tug_gate_out_within_the_push_radius_is_taxi_not_a_pushback() {
        let mut st = TaxiObsState::default();
        // #285: a GA aircraft taxis forward 6 kt to a nearby hold short — inside PUSH_MAX_M and
        // under GS_START, so the old distance rule called it a push, a hold, and a 15 s taxi.
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0, 0),
                (15, 40.0, 6, 0, 0),
                (30, 80.0, 6, 0, 0),
                (45, 120.0, 6, 0, 0),
                // Waiting at the hold short.
                (60, 120.0, 0, 0, 0),
                (75, 120.0, 0, 0, 0),
                (90, 120.0, 0, 0, 0),
                (105, 120.0, 0, 0, 0),
                (120, 900.0, 80, 400, 0),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, None);
        assert_eq!(obs[0].startup_sec, None);
        assert_eq!(obs[0].taxi_sec, 105); // 15 → 120, the hold included
    }

    #[test]
    fn a_remote_stand_tow_past_the_push_radius_is_still_a_pushback() {
        let mut st = TaxiObsState::default();
        // #285: a 420 m tow runs against the heading the whole way, so distance no longer ends it.
        let mut track: Vec<(i64, f64, i64, i64, i64)> = vec![(0, 0.0, 0, 0, 0)];
        for i in 1..=14 {
            track.push((i * 15, i as f64 * 30.0, 3, 0, 180));
        }
        // Tug disconnect at t=225, then a forward taxi from t=270.
        for i in 0..3 {
            track.push((225 + i * 15, 420.0, 0, 0, 180));
        }
        track.extend([
            (270, 500.0, 12, 0, 0),
            (285, 600.0, 15, 0, 0),
            (300, 700.0, 15, 0, 0),
            (390, 1500.0, 80, 400, 0),
        ]);

        let obs = run_track(&mut st, &track);

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, Some(210)); // 15 → 225, the whole tow
        assert_eq!(obs[0].startup_sec, Some(45)); // 225 → 270
        assert_eq!(obs[0].taxi_sec, 120); // 270 → 390
    }

    #[test]
    fn an_evenly_split_burst_falls_back_to_the_push_radius() {
        let mut st = TaxiObsState::default();
        // #285: the aircraft shuffles to and fro on the stand (nose steady), so equal travel runs
        // each way and the heading can't classify the burst — the distance proxy decides, and
        // inside PUSH_MAX_M that's a push.
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0, 180),
                (15, 8.0, 2, 0, 180),
                (30, 0.0, 2, 0, 180),
                (45, 8.0, 2, 0, 180),
                (60, 0.0, 2, 0, 180),
                (75, 0.0, 0, 0, 180),
                (90, 0.0, 0, 0, 180),
                (105, 0.0, 0, 0, 180),
                (120, 100.0, 12, 0, 0),
                (135, 200.0, 15, 0, 0),
                (150, 300.0, 15, 0, 0),
                (270, 1500.0, 80, 400, 0),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, Some(60)); // 15 → 75
        assert_eq!(obs[0].startup_sec, Some(45)); // 75 → 120
        assert_eq!(obs[0].taxi_sec, 150); // 120 → 270
    }

    #[test]
    fn a_two_update_shuffle_is_not_a_confirmed_push() {
        let mut st = TaxiObsState::default();
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0, 0),
                // Two moving updates covering 30 m — one short of confirmation.
                (15, 15.0, 2, 0, 0),
                (30, 30.0, 2, 0, 0),
                (45, 30.0, 0, 0, 0),
                (60, 30.0, 0, 0, 0),
                (75, 30.0, 0, 0, 0),
                (90, 30.0, 0, 0, 0),
                (105, 80.0, 12, 0, 0),
                (120, 150.0, 15, 0, 0),
                (135, 220.0, 15, 0, 0),
                (255, 900.0, 80, 400, 0),
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
            &[
                (0, 0.0, 0, 0, 0),
                (15, 0.0, 0, 150, 0),
                (30, 0.0, 0, 200, 0),
            ],
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
                (0, 0.0, 0, 0, 0),
                (15, 0.0, 0, 0, 0),
                // Two moving updates — the taxi burst isn't confirmed yet when it lifts off.
                (30, 40.0, 10, 0, 0),
                (45, 120.0, 20, 0, 0),
                (60, 900.0, 80, 400, 0),
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
                (0, 0.0, 0, 0, 0),
                // The taxi burst reports 150 ft above the first-seen altitude (sloped field).
                (15, 60.0, 12, 150, 0),
                (30, 130.0, 15, 150, 0),
                (45, 200.0, 15, 150, 0),
                (60, 270.0, 15, 160, 0),
            ],
        );
        assert!(
            taxiing.is_empty(),
            "a 10 ft change while taxiing isn't a takeoff"
        );

        let obs = run_track(&mut st, &[(180, 900.0, 80, 600, 0)]);
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
        process(
            &mut st,
            &ap,
            &rw,
            &one_with_arrival(40.0, 20, 0, "KAAA", "KBBB"),
            t(0),
        );

        assert!(st.dep.is_empty());
    }

    /// A nose-swinging push: the tug rolls the aircraft back while pivoting it from 180 through 85.
    /// The swing makes individual steps read forwards, which must not erase the push (#285 QA).
    #[test]
    fn a_push_that_swings_the_nose_is_still_a_push() {
        let mut st = TaxiObsState::default();
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0, 180),
                (15, 7.0, 2, 0, 180),
                (30, 14.0, 2, 0, 180),
                (45, 21.0, 2, 0, 170),
                (60, 28.0, 2, 0, 140),
                (75, 34.0, 2, 0, 110),
                (90, 40.0, 2, 0, 85),
                (105, 40.0, 0, 0, 85),
                (120, 40.0, 0, 0, 85),
                (135, 40.0, 0, 0, 85),
                (150, 40.0, 0, 0, 85),
                (165, 100.0, 12, 0, 0),
                (180, 190.0, 15, 0, 0),
                (195, 280.0, 15, 0, 0),
                (400, 900.0, 80, 400, 0),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, Some(90)); // 15 → 105
        assert_eq!(obs[0].startup_sec, Some(60)); // 105 → 165
        assert_eq!(obs[0].taxi_sec, 235); // 165 → 400
    }

    /// Push-and-pull: the tug pushes back, then pulls the aircraft forward to line it up. The pull
    /// ends the push where it began rather than discarding it (#285 QA).
    #[test]
    fn a_tug_pulling_forward_ends_the_push_instead_of_erasing_it() {
        let mut st = TaxiObsState::default();
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0, 180),
                (15, 8.0, 2, 0, 180),
                (30, 16.0, 2, 0, 180),
                (45, 24.0, 2, 0, 180),
                (60, 32.0, 2, 0, 180),
                // Pulled forward (south, along the nose) for two updates.
                (75, 24.0, 2, 0, 180),
                (90, 16.0, 2, 0, 180),
                (105, 16.0, 0, 0, 180),
                (120, 16.0, 0, 0, 180),
                (135, 16.0, 0, 0, 180),
                (150, -60.0, 12, 0, 180),
                (165, -150.0, 15, 0, 180),
                (180, -240.0, 15, 0, 180),
                (400, -900.0, 80, 400, 180),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, Some(60)); // 15 → 75, the pull
        assert_eq!(obs[0].taxi_sec, 325); // 75 → 400
    }

    /// Heading 350 with a due-north track is 10 deg off, not 350 — a forward taxi, not a push.
    #[test]
    fn a_heading_either_side_of_north_is_compared_the_short_way_round() {
        let mut st = TaxiObsState::default();
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0, 350),
                (15, 40.0, 6, 0, 350),
                (30, 80.0, 6, 0, 350),
                (45, 120.0, 6, 0, 350),
                (60, 120.0, 0, 0, 350),
                (75, 120.0, 0, 0, 350),
                (90, 120.0, 0, 0, 350),
                (105, 120.0, 0, 0, 350),
                (120, 900.0, 80, 400, 350),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, None);
        assert_eq!(obs[0].startup_sec, None);
    }

    /// The tie fallback cuts both ways: a burst the heading can't classify — here every step runs
    /// across the nose — is taxi once it leaves the push radius.
    #[test]
    fn a_burst_the_heading_cannot_classify_is_taxi_beyond_the_push_radius() {
        let mut st = TaxiObsState::default();
        let obs = run_track(
            &mut st,
            &[
                // Nose east, tracking north: every step is sideways, so nothing votes.
                (0, 0.0, 0, 0, 90),
                (15, 60.0, 5, 0, 90),
                (30, 120.0, 5, 0, 90),
                (45, 180.0, 5, 0, 90),
                (60, 240.0, 5, 0, 90),
                (75, 240.0, 0, 0, 90),
                (90, 240.0, 0, 0, 90),
                (105, 240.0, 0, 0, 90),
                (120, 900.0, 80, 400, 90),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, None);
        assert_eq!(obs[0].startup_sec, None);
        assert_eq!(obs[0].taxi_sec, 105); // 15 → 120
    }

    /// Jitter-sized steps must not outvote the tug: two 30 m backwards steps beat three 3 m
    /// forward ones, which a per-step count would get backwards (#285 QA).
    #[test]
    fn a_few_metres_of_jitter_do_not_outvote_the_tug() {
        let mut st = TaxiObsState::default();
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0, 180),
                (15, -3.0, 1, 0, 180),
                (30, -6.0, 1, 0, 180),
                (45, 24.0, 2, 0, 180),
                (60, 24.0, 0, 0, 180),
                (75, 24.0, 0, 0, 180),
                (90, 24.0, 0, 0, 180),
                (105, 24.0, 0, 0, 180),
                (120, -100.0, 12, 0, 180),
                (135, -200.0, 15, 0, 180),
                (150, -300.0, 15, 0, 180),
                (300, -900.0, 80, 400, 180),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, Some(45)); // 15 → 60
        assert_eq!(obs[0].startup_sec, Some(60)); // 60 → 120
    }

    /// A heading that never agrees with the track (stale or wrong) can't hold an aircraft in
    /// PushingBack for its whole ground phase: past the tow distance it's taxi, and the figures it
    /// would otherwise have claimed never reach the table (#285 QA).
    #[test]
    fn a_direction_push_that_runs_too_far_is_taxi() {
        let mut st = TaxiObsState::default();
        // 100 m per update for 25 updates — 2.5 km, well past the tow cap, inside the time cap.
        let mut track: Vec<(i64, f64, i64, i64, i64)> = vec![(0, 0.0, 0, 0, 180)];
        for i in 1..=25 {
            track.push((i * 15, i as f64 * 100.0, 5, 0, 180));
        }
        // Then a stop and a departure, which a runaway push would report as push + start-up.
        for i in 26..=30 {
            track.push((i * 15, 2500.0, 0, 0, 180));
        }
        track.push((465, 2600.0, 12, 0, 180));
        track.push((480, 2800.0, 15, 0, 180));
        track.push((600, 4000.0, 80, 400, 180));
        let obs = run_track(&mut st, &track);

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, None);
        assert_eq!(obs[0].startup_sec, None);
    }

    #[test]
    fn a_direction_push_that_runs_too_long_is_taxi() {
        let mut st = TaxiObsState::default();
        // 10 m per update stays inside the tow distance, but 90 updates is 22 minutes of "push".
        let mut track: Vec<(i64, f64, i64, i64, i64)> = vec![(0, 0.0, 0, 0, 180)];
        for i in 1..=90 {
            track.push((i * 15, i as f64 * 10.0, 2, 0, 180));
        }
        for i in 91..=95 {
            track.push((i * 15, 900.0, 0, 0, 180));
        }
        track.push((1440, 1000.0, 12, 0, 180));
        track.push((1455, 1200.0, 15, 0, 180));
        track.push((1600, 2000.0, 80, 400, 180));
        let obs = run_track(&mut st, &track);

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, None);
        assert_eq!(obs[0].startup_sec, None);
    }

    /// The nose swings steadily through the push, so two updates in a row track across it rather
    /// than against it. Those sideways steps must not end the push (#285 QA).
    #[test]
    fn a_push_tracking_across_the_nose_is_still_a_push() {
        let mut st = TaxiObsState::default();
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0, 180),
                (15, 8.0, 2, 0, 175),
                (30, 16.0, 2, 0, 160),
                (45, 24.0, 2, 0, 125),
                // mid-heading over these two steps is ~115 and ~100 deg off the northward track.
                (60, 31.0, 2, 0, 105),
                (75, 38.0, 2, 0, 95),
                (90, 38.0, 0, 0, 95),
                (105, 38.0, 0, 0, 95),
                (120, 38.0, 0, 0, 95),
                (135, 38.0, 0, 0, 95),
                (150, 100.0, 12, 0, 0),
                (165, 200.0, 15, 0, 0),
                (180, 300.0, 15, 0, 0),
                (400, 900.0, 80, 400, 0),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, Some(75)); // 15 -> 90
        assert_eq!(obs[0].startup_sec, Some(60)); // 90 -> 150
    }

    /// A brief pause earlier in the push isn't the boundary for a pull that comes later: the push
    /// ends where the aircraft actually started running forwards (#285 QA).
    #[test]
    fn a_stale_pause_does_not_become_the_push_stop() {
        let mut st = TaxiObsState::default();
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0, 180),
                (15, 8.0, 2, 0, 180),
                (30, 16.0, 2, 0, 180),
                (45, 24.0, 2, 0, 180),
                // One still update mid-push (tug repositioning), then pushing again.
                (60, 24.0, 0, 0, 180),
                (75, 32.0, 2, 0, 180),
                (90, 40.0, 2, 0, 180),
                // Now pulled forward: the push ends here, not back at t=60.
                (105, 32.0, 2, 0, 180),
                (120, 24.0, 2, 0, 180),
                (135, -60.0, 12, 0, 180),
                (150, -160.0, 15, 0, 180),
                (300, -900.0, 80, 400, 180),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, Some(90)); // 15 -> 105, not 15 -> 60
        assert_eq!(obs[0].startup_sec, Some(0));
    }

    /// A burst the heading couldn't classify is governed by the distance proxy, so a forward step
    /// mustn't end it — the direction signal was already judged unusable here (#285 QA).
    #[test]
    fn a_fallback_classified_push_is_not_ended_by_a_forward_step() {
        let mut st = TaxiObsState::default();
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0, 180),
                // 8 m + 8 m backwards against 16 m forwards: evenly split, so the distance proxy
                // made this a push. Two forward steps now must not cut it short.
                (15, 8.0, 2, 0, 180),
                (30, 16.0, 2, 0, 180),
                (45, 0.0, 2, 0, 180),
                (60, -6.0, 2, 0, 180),
                (75, -12.0, 2, 0, 180),
                (90, -12.0, 0, 0, 180),
                (105, -12.0, 0, 0, 180),
                (120, -12.0, 0, 0, 180),
                (135, -12.0, 0, 0, 180),
                (150, -100.0, 12, 0, 180),
                (165, -200.0, 15, 0, 180),
                (300, -900.0, 80, 400, 180),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, Some(75)); // 15 -> 90, the confirmed stop
        assert_eq!(obs[0].startup_sec, Some(60)); // 90 -> 150
    }

    /// A start-up gap longer than the estimator's bounds is a misread track, not a slow tug — the
    /// row keeps its taxi figure and drops the absurd one (#285 QA).
    #[test]
    fn an_over_long_start_up_is_not_stored() {
        let mut st = TaxiObsState::default();
        let mut track: Vec<(i64, f64, i64, i64, i64)> = vec![
            (0, 0.0, 0, 0, 180),
            (15, 8.0, 2, 0, 180),
            (30, 16.0, 2, 0, 180),
            (45, 24.0, 2, 0, 180),
        ];
        // Sat at the push point for 20 minutes.
        for i in 4..=84 {
            track.push((i * 15, 24.0, 0, 0, 180));
        }
        track.push((1275, -60.0, 12, 0, 180));
        track.push((1290, -160.0, 15, 0, 180));
        track.push((1305, -260.0, 15, 0, 180));
        track.push((1500, -900.0, 80, 400, 180));
        let obs = run_track(&mut st, &track);

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, Some(45)); // 15 -> 60
        assert_eq!(
            obs[0].startup_sec, None,
            "1215 s is past STARTUP_BOUNDS_SEC"
        );
        assert_eq!(obs[0].taxi_sec, 225); // 1275 -> 1500
    }

    #[test]
    fn an_absurd_phase_duration_is_dropped_rather_than_stored() {
        assert_eq!(sane_sec(600, PUSHBACK_BOUNDS_SEC), Some(600));
        assert_eq!(sane_sec(-1, PUSHBACK_BOUNDS_SEC), None);
        assert_eq!(sane_sec(4000, PUSHBACK_BOUNDS_SEC), None);
        assert_eq!(sane_sec(1000, STARTUP_BOUNDS_SEC), None);
    }
}
