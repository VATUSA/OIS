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
//! A push is the first burst while it stays below [`GS_START`] and runs *against* the reported
//! heading — a tug pushes the aircraft backwards, taxi runs along the nose (#285). Steps vote by
//! how far they travelled, so jitter can't outweigh a tug step, and each step is judged against the
//! heading at its *start*. Where the heading gives no answer (no moving step, or an even split) the
//! older distance proxy decides: within [`PUSH_MAX_M`] of where the aircraft was parked.
//!
//! The push ends where the nose turns forward: [`FLIP_CONFIRM_UPDATES`] consecutive forward steps
//! close it *there* and start the taxi, rather than discarding it — a curved push (the nose swinging
//! as the tug rolls it back) and a push-and-pull both keep their pushback. A direction-classified
//! push is still bounded by [`PUSH_FAR_M`] and [`PUSH_MAX_SEC`], so a stale heading can't hold a
//! session in the push phase. A burst that breaks those limits is taxi — a powerback, a no-tug
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
/// How far a step's track must run from the reported heading to count as moving backwards (a tug
/// push) rather than forwards (taxi). Well clear of 90°, which is where a nose-swinging push sits.
const BACKWARDS_DEG: f64 = 120.0;
/// Consecutive forward steps that end a push (the nose has turned into the taxi).
const FLIP_CONFIRM_UPDATES: u32 = 2;
/// Travel a step must carry to vote on ending a push. `MOVE_M` is only "not stationary"; a burst
/// needs [`BURST_MIN_M`] over [`CONFIRM_UPDATES`] steps to confirm, so hold a flip step to the same
/// per-step share and GPS wobble can't close a push (#285).
const FLIP_MIN_STEP_M: f64 = BURST_MIN_M / CONFIRM_UPDATES as f64;
/// Backstops on a push the heading classified: past either, the burst is taxi however it reads.
const PUSH_FAR_M: f64 = 800.0;
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
    /// Metres travelled against the reported heading, and along it (#285). Distance, not step count:
    /// a [`MOVE_M`] jitter step shouldn't outvote a tug step an order of magnitude longer.
    back_m: f64,
    fwd_m: f64,
}

impl Run {
    /// Whether this burst is a push by direction: more of its travel runs against the heading than
    /// along it, by more than a single jitter step. `None` when the steps can't say (nothing moved,
    /// or the two are within [`MOVE_M`] of each other) — the caller falls back to the distance
    /// proxy. The margin matters: without it, float noise across equal-length steps decides.
    fn backwards(&self) -> Option<bool> {
        let margin = self.back_m - self.fwd_m;
        (margin.abs() > MOVE_M).then_some(margin > 0.0)
    }
}

/// Whether a step from `from` to `to` runs against `hdg` — the aircraft moving backwards. `hdg` is
/// the heading at the *start* of the step: sampling the end biases every comparison of a turning
/// aircraft by roughly half the turn.
fn moved_backwards(from: (f64, f64), to: (f64, f64), hdg: i64) -> bool {
    let track = bearing_deg([from.0, from.1], [to.0, to.1]);
    let off = (track - hdg as f64).abs() % 360.0;
    // Fold onto 0..=180 so a heading either side of north compares correctly (e.g. 350 vs 000).
    let off = if off > 180.0 { 360.0 - off } else { off };
    off > BACKWARDS_DEG
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
    /// Heading reported at the previous update: a step is judged by where the nose pointed when it
    /// began, not where it ended up.
    last_hdg: i64,
    /// `(first forward step, how many have followed)` while pushing back — the candidate end of the
    /// push, confirmed at [`FLIP_CONFIRM_UPDATES`].
    fwd_flip: Option<(i64, u32)>,
    base_alt: i64,
}

impl Session {
    /// Feed one on-ground update into the phase machine.
    fn advance(&mut self, now_ms: i64, lat: f64, lon: f64, gs: i64, alt: i64, hdg: i64) {
        let step_m = gc_dist(self.last_lat, self.last_lon, lat, lon) * M_PER_NM;
        let moving = step_m >= MOVE_M;
        let backwards =
            moving && moved_backwards((self.last_lat, self.last_lon), (lat, lon), self.last_hdg);
        (self.last_lat, self.last_lon) = (lat, lon);
        self.last_hdg = hdg;
        let from_stand_m = gc_dist(self.first_lat, self.first_lon, lat, lon) * M_PER_NM;

        // A push that carries on backwards after a brief stop has left that stop behind: it is no
        // longer where the push ended, and splitting there would book minutes of real pushing as
        // taxi (#285). Both consumers of `last_pause` — the flip below and the speed/bounds exit —
        // then only ever see a pause the push actually finished at. The resume step itself is
        // exempt: it is judged against the heading from before the stop.
        if self.phase == Phase::PushingBack
            && backwards
            && step_m >= FLIP_MIN_STEP_M
            && self.last_pause.is_some_and(|(_, resume)| resume != now_ms)
        {
            self.last_pause = None;
        }

        // The nose has turned into the taxi: confirmed forward movement ends the push *here* rather
        // than discarding it, so a curved push or a push-and-pull keeps its pushback (#285). Only a
        // step carrying real distance votes — `Run::backwards()` already refuses to let a `MOVE_M`
        // wobble outvote a tug step, and ending a push is at least as consequential as classifying
        // one: two jitter steps while the tug is disconnecting must not close the push and charge
        // the whole start-up to taxi.
        if self.phase == Phase::PushingBack && self.push_by_direction {
            let flip = match (step_m >= FLIP_MIN_STEP_M && !backwards, self.fwd_flip) {
                (true, Some((start, seen))) => Some((start, seen + 1)),
                (true, None) => Some((now_ms, 1)),
                (false, _) => None,
            };
            self.fwd_flip = flip;
            if let Some((start, seen)) = flip
                && seen >= FLIP_CONFIRM_UPDATES
            {
                // A brief stop before the nose turned is the real push end, with the start-up gap
                // after it; without one, the push ran straight into the taxi at the flip. A pause
                // the push then carried on past has already been cleared below, so whatever is here
                // is still the end of the push.
                let (stop, resume) = self.last_pause.unwrap_or((start, start));
                self.push_stop_ms = Some(stop);
                self.taxi_start_ms = Some(resume);
                self.start_taxi(alt);
                return;
            }
        }

        // A "push" that reaches taxi speed, or outruns the bounds on a push — the distance proxy's
        // [`PUSH_MAX_M`] when it did the classifying, else the far backstops — is taxiing: it splits
        // at its last pause (a stop still pending now, or an earlier brief one), else the whole
        // burst was the taxi.
        let outran_push = if self.push_by_direction {
            from_stand_m > PUSH_FAR_M
                || self
                    .push_start_ms
                    .is_some_and(|start| now_ms - start > PUSH_MAX_SEC * 1000)
        } else {
            from_stand_m > PUSH_MAX_M
        };
        let left_the_push = gs > GS_START || outran_push;
        if self.phase == Phase::PushingBack && left_the_push {
            let pause = self.run.map(|r| (r.start_ms, now_ms)).or(self.last_pause);
            match pause {
                Some((stop, resume)) => {
                    self.push_stop_ms = Some(stop);
                    self.taxi_start_ms = Some(resume);
                }
                // No pause to split at. A push the heading identified is real, so it simply ends
                // here and the taxi takes over (#285); one the distance proxy guessed at was never
                // a push, so the whole burst becomes the taxi (#277).
                None if self.push_by_direction && !outran_push => {
                    self.push_stop_ms = Some(now_ms);
                    self.taxi_start_ms = Some(now_ms);
                }
                None => self.taxi_start_ms = self.push_start_ms.take(),
            }
            self.start_taxi(alt);
            return;
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
        if moving {
            if backwards {
                run.back_m += step_m;
            } else {
                run.fwd_m += step_m;
            }
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
        self.fwd_flip = None;
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
            // An implausible phase duration means the machine misread the session, so the whole
            // observation goes rather than a metric being nulled — a null now means "no push at
            // all" to the estimator (#287), which would be a lie here.
            let in_bounds = |v: Option<i32>, (lo, hi): (f64, f64)| {
                v.is_none_or(|v| (lo..=hi).contains(&(v as f64)))
            };
            if (MIN_TAXI_SEC..=MAX_TAXI_SEC).contains(&dur)
                && in_bounds(pushback_sec, PUSHBACK_BOUNDS_SEC)
                && in_bounds(startup_sec, STARTUP_BOUNDS_SEC)
            {
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
                        last_hdg: p.heading,
                        fwd_flip: None,
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
                // Confirmed as a push at t=60 by the distance proxy: the steps alternate, so the
                // heading can't classify the burst (#285), and it stays inside PUSH_MAX_M.
                (15, 6.0, 2, 0, 180),
                (30, 12.0, 2, 0, 0),
                (45, 18.0, 2, 0, 180),
                (60, 24.0, 2, 0, 0),
                // ... but it keeps accelerating past 7 kt without stopping: it was the taxi all
                // along, and a push the heading never confirmed is discarded, not split.
                (75, 80.0, 12, 0, 0),
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
                (0, 0.0, 0, 0, 0),
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
    fn a_curved_push_with_the_nose_swinging_keeps_its_pushback() {
        let mut st = TaxiObsState::default();
        // #285 QA: a tug rolls the aircraft back while the nose swings round. Every step still runs
        // against the heading it began with, so the push survives and ends at its stop.
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0, 180),
                (15, 20.0, 2, 0, 180),
                (30, 40.0, 2, 0, 180),
                (45, 60.0, 2, 0, 170),
                (60, 80.0, 2, 0, 140),
                (75, 100.0, 2, 0, 110),
                // Tug disconnect: stopped long enough to confirm the push stop, then start-up.
                (90, 100.0, 0, 0, 85),
                (105, 100.0, 0, 0, 85),
                (120, 100.0, 0, 0, 85),
                (150, 100.0, 0, 0, 85),
                // Taxi away.
                (165, 160.0, 12, 0, 0),
                (180, 260.0, 15, 0, 0),
                (195, 360.0, 15, 0, 0),
                (400, 1500.0, 80, 400, 0),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, Some(75)); // 15 → 90
        assert_eq!(obs[0].startup_sec, Some(75)); // 90 → 165
        assert_eq!(obs[0].taxi_sec, 235); // 165 → 400
    }

    #[test]
    fn a_push_then_a_pivot_while_still_rolling_keeps_its_pushback() {
        let mut st = TaxiObsState::default();
        // #285 QA: straight push, then a 90° pivot with the aircraft still moving — the nose turning
        // forward closes the push there rather than discarding it.
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0, 180),
                (15, 20.0, 2, 0, 180),
                (30, 40.0, 2, 0, 180),
                (45, 60.0, 2, 0, 180),
                (60, 90.0, 3, 0, 135),
                (75, 130.0, 4, 0, 90),
                (90, 200.0, 12, 0, 0),
                (105, 300.0, 15, 0, 0),
                (300, 1500.0, 80, 400, 0),
            ],
        );

        assert_eq!(obs.len(), 1);
        // Still rolling back through the pivot (each step judged on the heading it began with), so
        // the push runs 15 → 90, where taxi speed takes over; no stop between them, so no start-up.
        assert_eq!(obs[0].pushback_sec, Some(75));
        assert_eq!(obs[0].startup_sec, Some(0));
        assert_eq!(obs[0].taxi_sec, 210); // 90 → 300
    }

    /// A push that carries on backwards past a brief stop did not end at that stop. Before #285's
    /// rework the flip split there anyway, booking minutes of real pushing as taxi: this track
    /// recorded `45 / 15 / 625` instead of the push it actually flew.
    #[test]
    fn a_push_that_carries_on_past_a_brief_stop_is_not_split_there() {
        let mut track: Vec<(i64, f64, i64, i64, i64)> = vec![(0, 0.0, 0, 0, 0)];
        let mut m = 0.0;
        // Push confirms t=15..45.
        for i in 1..=3 {
            m += 25.0;
            track.push((15 * i, m, 2, 0, 180));
        }
        // One-poll hesitation — too short to confirm a stop.
        track.push((60, m, 0, 0, 180));
        // ... and the tug keeps pushing for another two and a half minutes.
        for i in 5..=15 {
            m += 25.0;
            track.push((15 * i, m, 2, 0, 180));
        }
        // The nose finally swings into the taxi.
        for i in 0..5 {
            m += 20.0;
            track.push((240 + 15 * i, m, 2, 0, 20));
        }
        for i in 0..3 {
            track.push((330 + 15 * i, m, 0, 0, 20));
        }
        track.push((375, m + 160.0, 15, 0, 20));
        track.push((700, m + 2500.0, 80, 400, 20));

        let mut st = TaxiObsState::default();
        let obs = run_track(&mut st, &track);
        assert_eq!(obs.len(), 1);
        // The push ran to the flip at t=255, not to the stale hesitation at t=60.
        assert_eq!(obs[0].pushback_sec, Some(240));
    }

    /// GPS wobble while the tug disconnects must not end the push: two `MOVE_M`-sized steps that
    /// happen to read forward used to close it and charge the whole start-up to taxi (`45 / 15 /
    /// 525` on this track).
    #[test]
    fn jitter_after_the_tug_stops_does_not_end_the_push() {
        let mut track: Vec<(i64, f64, i64, i64, i64)> = vec![
            (0, 0.0, 0, 0, 0),
            (15, 20.0, 2, 0, 180),
            (30, 40.0, 2, 0, 180),
            (45, 60.0, 2, 0, 180),
            (60, 60.0, 0, 0, 180), // tug stops
            (75, 57.5, 0, 0, 180), // 2.5 m of wobble that reads "forward"
            (90, 55.0, 0, 0, 180), // and again — two in a row
        ];
        // Genuinely stationary for four minutes: engine start and tug disconnect.
        let mut secs = 105;
        while secs <= 300 {
            track.push((secs, 55.0, 0, 0, 180));
            secs += 15;
        }
        track.push((315, 215.0, 15, 0, 0));
        track.push((330, 375.0, 15, 0, 0));
        track.push((600, 2000.0, 80, 400, 0));

        let mut st = TaxiObsState::default();
        let obs = run_track(&mut st, &track);
        assert_eq!(obs.len(), 1);
        // The start-up gap is measured, not swallowed by the taxi.
        assert_eq!(obs[0].startup_sec, Some(210));
        assert_eq!(obs[0].taxi_sec, 285);
    }

    /// `BACKWARDS_DEG` sits at 120°, not the 90° a pivoting push's geometry hovers around. These
    /// steps track 105° off the nose — forward at 120, backwards at 90 — so the threshold's value is
    /// pinned, not merely its existence. At 90 this records `90 / 45 / 150`.
    #[test]
    fn a_step_across_the_nose_is_not_backwards_at_the_120_degree_threshold() {
        let mut st = TaxiObsState::default();
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0, 0),
                (15, 25.0, 2, 0, 180),
                (30, 50.0, 2, 0, 180),
                (45, 75.0, 2, 0, 105),
                (60, 100.0, 2, 0, 105),
                (75, 125.0, 2, 0, 105),
                (90, 150.0, 2, 0, 105),
                (105, 150.0, 0, 0, 105),
                (120, 150.0, 0, 0, 105),
                (135, 150.0, 0, 0, 105),
                (150, 280.0, 15, 0, 0),
                (165, 420.0, 15, 0, 0),
                (300, 1400.0, 80, 400, 0),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, Some(45));
        assert_eq!(obs[0].taxi_sec, 240);
    }

    /// The forward-flip exit is gated on `push_by_direction`: a burst the heading could not classify
    /// was guessed at by the distance proxy, so a heading signal must not end it either. Without the
    /// gate this records `60 / 0 / 225`.
    #[test]
    fn forward_steps_do_not_end_a_push_the_distance_proxy_classified() {
        let mut st = TaxiObsState::default();
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0, 0),
                // Alternating headings: back and forward travel tie, so the vote abstains and the
                // distance proxy confirms the push (inside PUSH_MAX_M).
                (15, 6.0, 2, 0, 180),
                (30, 12.0, 2, 0, 0),
                (45, 18.0, 2, 0, 180),
                (60, 24.0, 2, 0, 0),
                // Two unambiguous forward steps — they must NOT end this push.
                (75, 50.0, 2, 0, 0),
                (90, 76.0, 2, 0, 0),
                (105, 100.0, 2, 0, 0),
                (120, 100.0, 0, 0, 0),
                (135, 100.0, 0, 0, 0),
                (150, 100.0, 0, 0, 0),
                (165, 260.0, 15, 0, 0),
                (300, 1400.0, 80, 400, 0),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, Some(105)); // ended by the stop, not the flip
        assert_eq!(obs[0].startup_sec, Some(45));
        assert_eq!(obs[0].taxi_sec, 135);
    }

    /// Push-and-pull: the tug pushes the aircraft back, then pulls it forward to line it up before
    /// disconnecting. The confirmed forward run ends the push there rather than discarding it.
    #[test]
    fn a_push_and_pull_keeps_its_pushback() {
        let mut st = TaxiObsState::default();
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0, 0),
                (15, 25.0, 2, 0, 180),
                (30, 50.0, 2, 0, 180),
                (45, 75.0, 2, 0, 180),
                (60, 100.0, 2, 0, 180),
                // Pulled forward again to line up: the track now runs along the nose.
                (75, 80.0, 2, 0, 180),
                (90, 60.0, 2, 0, 180),
                (105, 40.0, 2, 0, 180),
                (120, 40.0, 0, 0, 0),
                (135, 40.0, 0, 0, 0),
                (150, 40.0, 0, 0, 0),
                (165, 200.0, 15, 0, 0),
                (300, 1400.0, 80, 400, 0),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, Some(60)); // kept, and ended at the flip
        assert!(obs[0].startup_sec.is_some());
    }

    #[test]
    fn a_taxi_slightly_off_its_heading_is_not_a_push() {
        let mut st = TaxiObsState::default();
        // Kills BACKWARDS_DEG -> 0: a forward taxi is never exactly on its reported heading.
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0, 10),
                (15, 40.0, 6, 0, 10),
                (30, 80.0, 6, 0, 10),
                (45, 120.0, 6, 0, 10),
                (60, 120.0, 0, 0, 10),
                (75, 120.0, 0, 0, 10),
                (90, 120.0, 0, 0, 10),
                (150, 900.0, 80, 400, 10),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, None);
        assert_eq!(obs[0].taxi_sec, 135); // 15 → 150
    }

    #[test]
    fn a_heading_either_side_of_north_compares_as_forward() {
        let mut st = TaxiObsState::default();
        // Kills dropping the wraparound fold: heading 350 with a due-north track is 10° off, not 350.
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
                (150, 900.0, 80, 400, 350),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, None);
        assert_eq!(obs[0].taxi_sec, 135);
    }

    #[test]
    fn a_burst_running_mostly_along_the_nose_is_taxi_not_a_push() {
        let mut st = TaxiObsState::default();
        // The reported heading alternates, but each step is judged against the heading at its
        // *start*, so the burst confirms with 120 m along the nose against 60 m into it: the vote
        // reads forward and the burst is taxi. It does not tie, and the distance proxy is never
        // consulted — a genuine tie means balanced travel, which leaves the aircraft back near the
        // stand and so inside PUSH_MAX_M (that case is `an_evenly_split_burst_falls_back_to_the_
        // push_radius`).
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0, 0),
                (15, 60.0, 3, 0, 180),
                (30, 120.0, 3, 0, 0),
                (45, 180.0, 3, 0, 180),
                (60, 240.0, 3, 0, 0),
                (75, 240.0, 0, 0, 0),
                (90, 240.0, 0, 0, 0),
                (105, 240.0, 0, 0, 0),
                (165, 900.0, 80, 400, 0),
            ],
        );

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, None);
        assert_eq!(obs[0].startup_sec, None);
        assert_eq!(obs[0].taxi_sec, 150); // 15 → 165
    }

    #[test]
    fn jitter_cannot_outvote_the_tug_steps_that_moved_the_aircraft() {
        let mut st = TaxiObsState::default();
        // Kills the unweighted vote: three 2 m forward jitter steps against three 20 m tug steps.
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0, 180),
                // 20 m tug steps against 3 m of forward jitter — both above MOVE_M, so both vote.
                (15, 20.0, 2, 0, 180),
                (30, 17.0, 1, 0, 180),
                (45, 37.0, 2, 0, 180),
                (60, 34.0, 1, 0, 180),
                (75, 54.0, 2, 0, 180),
                (90, 54.0, 0, 0, 180),
                (105, 54.0, 0, 0, 180),
                (120, 54.0, 0, 0, 180),
                (135, 120.0, 12, 0, 0),
                (150, 220.0, 15, 0, 0),
                (165, 320.0, 15, 0, 0),
                (370, 1500.0, 80, 400, 0),
            ],
        );

        assert_eq!(obs.len(), 1);
        // 15 → 90, the confirmed stop: the tug steps win the vote, and a lone forward jitter step
        // between them is not a confirmed flip, so it doesn't end the push early.
        assert_eq!(obs[0].pushback_sec, Some(75));
        assert_eq!(obs[0].startup_sec, Some(45)); // 90 → 135
    }

    #[test]
    fn a_push_that_outruns_the_distance_cap_is_read_as_taxi() {
        let mut st = TaxiObsState::default();
        // Backwards-reading movement for 900 m in under 4 minutes: past PUSH_FAR_M, so the heading
        // no longer keeps it a push however convincing it looks.
        let mut track: Vec<(i64, f64, i64, i64, i64)> = vec![(0, 0.0, 0, 0, 180)];
        for i in 1..=15 {
            track.push((i * 15, i as f64 * 60.0, 5, 0, 180));
        }
        // Then it stops and taxis away normally — without the cap this would record the stop as a
        // push stop and bank a 3½-minute "pushback".
        for i in 0..3 {
            track.push((240 + i * 15, 900.0, 0, 0, 180));
        }
        track.extend([
            (300, 1000.0, 12, 0, 0),
            (315, 1100.0, 15, 0, 0),
            (330, 1200.0, 15, 0, 0),
            (500, 2500.0, 80, 400, 0),
        ]);

        let obs = run_track(&mut st, &track);

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, None);
        assert_eq!(obs[0].startup_sec, None);
        assert_eq!(obs[0].taxi_sec, 485); // 15 → 500
    }

    #[test]
    fn a_push_that_never_ends_is_bounded_and_read_as_taxi() {
        let mut st = TaxiObsState::default();
        // A stale heading would otherwise hold the session in PushingBack for hours: PUSH_MAX_SEC
        // (and PUSH_FAR_M) end it. 25 minutes of slow backwards-reading movement.
        let mut track: Vec<(i64, f64, i64, i64, i64)> = vec![(0, 0.0, 0, 0, 180)];
        for i in 1..=100 {
            track.push((i * 15, i as f64 * 7.0, 2, 0, 180));
        }
        // Stops and taxis away: without the duration cap this banks a 25-minute "pushback".
        for i in 0..3 {
            track.push((1515 + i * 15, 700.0, 0, 0, 180));
        }
        track.extend([
            (1575, 800.0, 12, 0, 0),
            (1590, 900.0, 15, 0, 0),
            (1605, 1000.0, 15, 0, 0),
            (1700, 2500.0, 80, 400, 0),
        ]);

        let obs = run_track(&mut st, &track);

        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].pushback_sec, None, "an unbounded push is not a push");
        assert!(obs[0].startup_sec.is_none());
        assert_eq!(obs[0].taxi_sec, 1685); // 15 → 1700
    }

    #[test]
    fn an_implausible_pushback_drops_the_whole_observation() {
        let mut st = TaxiObsState::default();
        // A burst the heading can't classify (alternating steps) stays a push on the distance proxy,
        // which has no duration cap — so it can shuffle on the stand for 23 minutes. The resulting
        // pushback is past PUSHBACK_BOUNDS_SEC, so the observation goes rather than being nulled.
        let mut track: Vec<(i64, f64, i64, i64, i64)> = vec![
            (0, 0.0, 0, 0, 0),
            (15, 6.0, 2, 0, 180),
            (30, 12.0, 2, 0, 0),
            (45, 18.0, 2, 0, 180),
            (60, 24.0, 2, 0, 0),
        ];
        for i in 0..90 {
            let m = if i % 2 == 0 { 27.0 } else { 24.0 };
            track.push((75 + i * 15, m, 2, 0, 0));
        }
        for i in 0..3 {
            track.push((1425 + i * 15, 24.0, 0, 0, 0));
        }
        track.extend([
            (1470, 100.0, 12, 0, 0),
            (1485, 200.0, 15, 0, 0),
            (1500, 300.0, 15, 0, 0),
            (1600, 1500.0, 80, 400, 0),
        ]);

        let obs = run_track(&mut st, &track);

        assert!(
            obs.is_empty(),
            "a >1200 s pushback is not a believable observation"
        );
    }

    #[test]
    fn an_implausible_start_up_drops_the_whole_observation() {
        let mut st = TaxiObsState::default();
        // A push, then a 20-minute wait before taxiing — past STARTUP_BOUNDS_SEC. Nulling the metric
        // would read as "no push at all" to the estimator (#287), so the row goes instead.
        let mut track: Vec<(i64, f64, i64, i64, i64)> = vec![
            (0, 0.0, 0, 0, 180),
            (15, 20.0, 2, 0, 180),
            (30, 40.0, 2, 0, 180),
            (45, 60.0, 2, 0, 180),
        ];
        for i in 0..70 {
            track.push((60 + i * 15, 60.0, 0, 0, 180));
        }
        track.extend([
            (1125, 160.0, 12, 0, 0),
            (1140, 260.0, 15, 0, 0),
            (1155, 360.0, 15, 0, 0),
            (1300, 2500.0, 80, 400, 0),
        ]);

        let obs = run_track(&mut st, &track);

        assert!(
            obs.is_empty(),
            "a >900 s start-up is not a believable observation"
        );
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
        // #285: steps alternate backwards/forwards (a shuffle on the stand), so the heading can't
        // classify the burst and the distance proxy decides — here, inside PUSH_MAX_M, a push.
        let obs = run_track(
            &mut st,
            &[
                (0, 0.0, 0, 0, 0),
                (15, 6.0, 2, 0, 180),
                (30, 12.0, 2, 0, 0),
                (45, 18.0, 2, 0, 180),
                (60, 24.0, 2, 0, 0),
                (75, 24.0, 0, 0, 0),
                (90, 24.0, 0, 0, 0),
                (105, 24.0, 0, 0, 0),
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
}
