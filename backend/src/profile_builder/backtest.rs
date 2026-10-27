//! Score a profile set against held-out flights through the live prediction path (#828).
//!
//! Each held-out flight is replayed from one instant: its first stored position at least 1,000 ft
//! above where it rolled. From there the live arrival predictor times it to the field
//! ([`predict::arrival_eta`]), and the same model times it to the point where it actually began its
//! descent ([`predict::eta_along_route`], the function FCA metering crosses fixes with). The errors
//! are predicted minus observed, so a positive number means the model was late.
//!
//! Only airborne instants are replayed, so the ground branch (the taxi allowance #814 is removing)
//! never enters the score.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use super::{
    is_held_out,
    samples::{CRUISE_BAND_FT, MAX_RATE_GAP_SEC, WindsTimeline},
    type_and_wake,
};
use crate::{
    errors::ApiError,
    feed::{
        airports::AirportDb,
        flow::gc_dist,
        nav::NavData,
        predict::{self, ArrivalInput},
        trajectory::{self, AircraftProfile, ProfileTable},
        winds::Winds,
    },
    repos::profile_history::{self, HistFlight, HistPlan, HistPosition},
};

/// The replay instant is the first position this far (ft) above the take-off roll.
pub const SNAPSHOT_CLIMB_FT: i32 = 1_000;
/// A flight whose first ground tick after flying is farther than this (nm) from its filed
/// destination diverted or disconnected somewhere else, and is not scored.
pub const ARRIVAL_RADIUS_NM: f64 = 5.0;

/// What a held-out flight actually did, and the instant it is replayed from.
#[derive(Debug, Clone)]
pub struct Observed {
    pub snapshot: HistPosition,
    /// The top-of-descent fix: the last position within [`CRUISE_BAND_FT`] of the flight's highest
    /// altitude before it descended to the field.
    pub tod: HistPosition,
    /// The last tick at or above the airborne groundspeed before the landing roll.
    pub touchdown: DateTime<Utc>,
}

/// Find the replay instant, the observed top of descent and the touchdown in one flight's positions
/// (oldest first). None when the flight can't be scored: it never climbed away, never landed at
/// `arr_ll`, or its descent or landing fall in a stretch too thinned to time.
pub fn observe(positions: &[HistPosition], arr_ll: [f64; 2]) -> Option<Observed> {
    let air = super::samples::AIRBORNE_GS_KT;
    let roll = positions.iter().position(|p| p.groundspeed >= air)?;
    let roll_alt = positions[roll].altitude;
    let snap = roll
        + positions[roll..]
            .iter()
            .position(|p| p.groundspeed >= air && p.altitude >= roll_alt + SNAPSHOT_CLIMB_FT)?;
    let land = snap + positions[snap..].iter().position(|p| p.groundspeed < air)?;
    let (last_air, ground) = (&positions[land - 1], &positions[land]);
    if (ground.ts - last_air.ts).num_seconds() > MAX_RATE_GAP_SEC
        || gc_dist(ground.lat as f64, ground.lon as f64, arr_ll[0], arr_ll[1]) > ARRIVAL_RADIUS_NM
    {
        return None;
    }
    let top = positions[snap..land].iter().map(|p| p.altitude).max()?;
    let tod = snap
        + positions[snap..land]
            .iter()
            .rposition(|p| (top - p.altitude) as f64 <= CRUISE_BAND_FT)?;
    // The fix after the TOD must be close enough to say when the descent began.
    let next = positions.get(tod + 1).filter(|_| tod + 1 < land)?;
    if tod == snap || (next.ts - positions[tod].ts).num_seconds() > MAX_RATE_GAP_SEC {
        return None;
    }
    Some(Observed {
        snapshot: positions[snap].clone(),
        tod: positions[tod].clone(),
        touchdown: last_air.ts,
    })
}

/// The filed plan a replay uses, as the live feed would have shown it at the replay instant.
pub struct Plan<'a> {
    pub dep: &'a str,
    pub arr: &'a str,
    pub route: &'a str,
    pub cruise_ft: f64,
    pub filed_tas: f64,
    pub arr_ll: [f64; 2],
}

/// The live predictor's input for an aircraft at `p`, built the way the arrival ladder builds it
/// (`feed::runway`): the filed cruise TAS capped by the profile.
pub fn arrival_input<'a>(
    plan: &Plan<'a>,
    p: &HistPosition,
    profile: &AircraftProfile,
) -> ArrivalInput<'a> {
    ArrivalInput {
        dep: plan.dep,
        arr: plan.arr,
        route: plan.route,
        pos: [p.lat as f64, p.lon as f64],
        alt_ft: p.altitude as f64,
        gs: p.groundspeed as i64,
        hdg: p.heading as i64,
        arr_ll: plan.arr_ll,
        cruise_ft: plan.cruise_ft,
        cruise_tas: trajectory::capped_cruise_tas(plan.filed_tas, plan.cruise_ft, profile),
    }
}

/// The ETA `along_nm` ahead of the aircraft on the route `arrival_eta` times it on: the same
/// [`predict::arrival_route`], the same [`Winds::along`] over it and the same
/// [`predict::eta_along_route`], with the target short of the field. At `along_nm` = the route
/// length it is `arrival_eta`'s ETA; `tod_eta_matches_arrival_eta_at_the_field` fails the moment the
/// two disagree.
pub fn eta_at_along(
    nav: &NavData,
    airports: &AirportDb,
    winds: &Winds,
    profile: &AircraftProfile,
    ac: &ArrivalInput,
    along_nm: f64,
    now: DateTime<Utc>,
) -> DateTime<Utc> {
    let route = predict::arrival_route(nav, airports, ac);
    let wind = winds.along(&route.path, route.route_nm);
    predict::eta_along_route(
        route.airborne,
        route.route_nm,
        along_nm.clamp(0.0, route.route_nm),
        ac.alt_ft,
        ac.gs as f64,
        ac.cruise_ft,
        ac.cruise_tas,
        airports,
        ac.arr,
        profile,
        wind,
        0.0,
        now,
    )
    .eta
}

/// Predicted arrival and top-of-descent times for one observed flight under `profile`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Prediction {
    pub arrival: DateTime<Utc>,
    pub tod: DateTime<Utc>,
}

pub fn predict_case(
    nav: &NavData,
    airports: &AirportDb,
    winds: &Winds,
    profile: &AircraftProfile,
    plan: &Plan,
    obs: &Observed,
) -> Prediction {
    let now = obs.snapshot.ts;
    let ac = arrival_input(plan, &obs.snapshot, profile);
    let arrival = predict::arrival_eta(nav, airports, winds, profile, &ac, 0.0, now).eta;
    // How far along the route the TOD fix lies: the route still to fly from here, less the route
    // still to fly from the TOD fix, both resolved the way arrival_eta resolves them.
    let here_nm = predict::arrival_route(nav, airports, &ac).route_nm;
    let at_tod = arrival_input(plan, &obs.tod, profile);
    let tod_nm = predict::arrival_route(nav, airports, &at_tod).route_nm;
    let tod = eta_at_along(nav, airports, winds, profile, &ac, here_nm - tod_nm, now);
    Prediction { arrival, tod }
}

/// Seconds of error (predicted − observed) for one flight under one profile set.
#[derive(Debug, Clone, Copy)]
pub struct CaseError {
    pub tod_sec: f64,
    pub arrival_sec: f64,
}

/// Error statistics over a list of signed errors (s).
#[derive(Debug, Clone, PartialEq)]
pub struct ErrorStats {
    pub n: usize,
    pub mae: f64,
    pub bias: f64,
    pub p90_abs: f64,
}

impl ErrorStats {
    pub fn of(errors: &[f64]) -> Self {
        let n = errors.len();
        if n == 0 {
            return Self {
                n,
                mae: 0.0,
                bias: 0.0,
                p90_abs: 0.0,
            };
        }
        let abs: Vec<f64> = errors.iter().map(|e| e.abs()).collect();
        Self {
            n,
            mae: abs.iter().sum::<f64>() / n as f64,
            bias: errors.iter().sum::<f64>() / n as f64,
            p90_abs: super::stats::quantile(&abs, 0.9).unwrap_or(0.0),
        }
    }
}

/// One scored flight: its type, whether set B profiles that type, and its error under each set.
pub struct Scored {
    pub ty: String,
    pub in_b: bool,
    pub a: CaseError,
    pub b: CaseError,
}

/// What a backtest replays against: the static reference data and the two profile sets.
pub struct Replay<'a> {
    pub nav: &'a NavData,
    pub airports: &'a AirportDb,
    pub set_a: &'a ProfileTable,
    pub set_b: &'a ProfileTable,
    /// The types set B changed.
    pub b_types: &'a BTreeSet<String>,
}

/// Replay the held-out flights wholly inside `[from, to)` under both sets. Flights are read in
/// batches of [`super::FLIGHTS_PER_BATCH`], each batch's positions, plan revisions and winds in one
/// query apiece. Returns the scored flights and the number held out.
pub async fn run(
    pool: &PgPool,
    replay: &Replay<'_>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<(Vec<Scored>, usize), ApiError> {
    let flights = profile_history::flights(pool, from, to).await?;
    let held: Vec<&HistFlight> = flights
        .iter()
        .filter(|f| is_held_out(f.session_id))
        .collect();
    let mut scored = Vec::new();
    for batch in held.chunks(super::FLIGHTS_PER_BATCH) {
        let ids: Vec<i64> = batch.iter().map(|f| f.session_id).collect();
        let positions = profile_history::positions(pool, &ids).await?;
        let plans = profile_history::plans(pool, &ids).await?;
        let (start, end) = super::batch_span(batch.iter().copied());
        let winds = profile_history::winds_covering(pool, start, end).await?;
        let timeline = WindsTimeline(&winds);
        for f in batch {
            let pos = positions.get(&f.session_id).map_or(&[][..], Vec::as_slice);
            let revs = plans.get(&f.session_id).map_or(&[][..], Vec::as_slice);
            if let Some(s) = score_flight(replay, f, pos, revs, &timeline) {
                scored.push(s);
            }
        }
    }
    Ok((scored, held.len()))
}

/// Score one held-out flight, or None when it cannot be scored (unknown destination, or
/// [`observe`] finds no replay instant, descent or landing to time).
fn score_flight(
    replay: &Replay,
    f: &HistFlight,
    positions: &[HistPosition],
    revisions: &[HistPlan],
    winds: &WindsTimeline,
) -> Option<Scored> {
    let arr = replay.airports.get(&f.arrival.to_ascii_uppercase())?;
    let obs = observe(positions, [arr.lat, arr.lon])?;
    // The plan in force at the replay instant (revisions are kept in stats.flight_plan); TAS has no
    // revision history, so it comes from the flight row.
    let rev = profile_history::plan_at(revisions, obs.snapshot.ts);
    let route = rev
        .and_then(|r| r.route.clone())
        .or_else(|| f.route.clone())
        .unwrap_or_default();
    let cruise_alt = rev.and_then(|r| r.cruise_alt).or(f.cruise_alt);
    let plan = Plan {
        dep: &f.departure,
        arr: &f.arrival,
        route: &route,
        cruise_ft: trajectory::parse_alt_ft(&cruise_alt.map(|a| a.to_string()).unwrap_or_default()),
        filed_tas: f.cruise_tas.unwrap_or(0) as f64,
        arr_ll: [arr.lat, arr.lon],
    };
    // The same freshness rule training uses: past WINDS_MAX_AGE_SEC the replay flies still air.
    let still = Winds::default();
    let field = winds.at(obs.snapshot.ts).unwrap_or(&still);
    let (ty, wake) = type_and_wake(f);
    let err = |profile: &AircraftProfile| {
        let p = predict_case(replay.nav, replay.airports, field, profile, &plan, &obs);
        CaseError {
            tod_sec: (p.tod - obs.tod.ts).num_milliseconds() as f64 / 1000.0,
            arrival_sec: (p.arrival - obs.touchdown).num_milliseconds() as f64 / 1000.0,
        }
    };
    Some(Scored {
        in_b: replay.b_types.contains(&ty),
        a: err(replay.set_a.resolve(&ty, &wake)),
        b: err(replay.set_b.resolve(&ty, &wake)),
        ty,
    })
}

/// Set A is the catalog as stored (`flow.aircraft_profile` rows); set B is set A with every type the
/// builder profiled replaced by its measurement, starting from the profile that type resolves to in
/// set A (its own row, else its wake class, else the default). Returns both tables and the types set
/// B changed.
pub fn profile_sets(
    rows: &[crate::models::AircraftProfileBody],
    built: &super::stats::ProfileSet,
) -> (
    ProfileTable,
    ProfileTable,
    std::collections::BTreeSet<String>,
) {
    use std::collections::HashMap;
    let mut default = AircraftProfile::default();
    let mut by_wake: HashMap<String, AircraftProfile> = HashMap::new();
    let mut by_type: HashMap<String, AircraftProfile> = HashMap::new();
    for r in rows {
        let p = crate::repos::aircraft_profiles::to_profile(r);
        match r.kind.as_str() {
            "default" => default = p,
            "wake" => {
                by_wake.insert(r.key.to_ascii_uppercase(), p);
            }
            _ => {
                by_type.insert(r.key.to_ascii_uppercase(), p);
            }
        }
    }
    let a = ProfileTable::new(default.clone(), by_wake.clone(), by_type.clone());
    let mut b_types = std::collections::BTreeSet::new();
    let mut b_by_type = by_type;
    for (ty, tp) in built.types.iter().filter(|(_, tp)| tp.profiled) {
        let base = a.resolve(ty, tp.wake.as_deref().unwrap_or("")).clone();
        b_by_type.insert(ty.to_ascii_uppercase(), tp.apply_to(&base));
        b_types.insert(ty.clone());
    }
    (a, ProfileTable::new(default, by_wake, b_by_type), b_types)
}

/// The backtest result as Markdown. Aggregates only.
pub fn report(scored: &[Scored], held_out: usize, label_a: &str, label_b: &str) -> String {
    let mut out = String::from("# Profile backtest\n\n");
    out.push_str(&format!(
        "{} held-out flights, {} scored (climbed away, descended and landed at the filed \
         destination with timeable positions). Error is predicted minus observed, in seconds; \
         positive means the model was late.\n\n",
        held_out,
        scored.len()
    ));
    let table = |out: &mut String, title: &str, rows: &[&Scored]| {
        out.push_str(&format!("## {title} ({} flights)\n\n", rows.len()));
        out.push_str("| Set | Point | MAE s | Bias s | p90 abs s |\n|---|---|---|---|---|\n");
        for (label, pick) in [
            (label_a, (|s: &Scored| s.a) as fn(&Scored) -> CaseError),
            (label_b, |s: &Scored| s.b),
        ] {
            for (point, get) in [
                ("TOD", (|e: CaseError| e.tod_sec) as fn(CaseError) -> f64),
                ("arrival", |e: CaseError| e.arrival_sec),
            ] {
                let v: Vec<f64> = rows.iter().map(|s| get(pick(s))).collect();
                let st = ErrorStats::of(&v);
                out.push_str(&format!(
                    "| {label} | {point} | {:.0} | {:+.0} | {:.0} |\n",
                    st.mae, st.bias, st.p90_abs
                ));
            }
        }
        out.push('\n');
    };
    let all: Vec<&Scored> = scored.iter().collect();
    let in_b: Vec<&Scored> = scored.iter().filter(|s| s.in_b).collect();
    table(&mut out, "All scored flights", &all);
    table(&mut out, "Flights of types the new set profiles", &in_b);
    out
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::Duration;

    use super::*;
    use crate::feed::airports::Airport;

    fn t0() -> DateTime<Utc> {
        DateTime::from_timestamp(1_790_000_000, 0).unwrap()
    }

    fn pos(sec: i64, lat: f64, lon: f64, alt: i32, gs: i16) -> HistPosition {
        HistPosition {
            ts: t0() + Duration::seconds(sec),
            lat: lat as f32,
            lon: lon as f32,
            altitude: alt,
            groundspeed: gs,
            heading: 220,
        }
    }

    /// Winds that change with altitude, so timing the route at the wrong altitude moves the ETA.
    fn layered_winds() -> Winds {
        serde_json::from_value(serde_json::json!({
            "stations": [
                {"lat": 40.0, "lon": -74.5, "levels": [[3000, {"dir": 220.0, "spd": 10.0}],
                                                       [24000, {"dir": 230.0, "spd": 90.0}]]},
                {"lat": 39.0, "lon": -76.5, "levels": [[3000, {"dir": 200.0, "spd": 15.0}],
                                                       [24000, {"dir": 240.0, "spd": 70.0}]]}
            ]
        }))
        .unwrap()
    }

    fn airports() -> AirportDb {
        HashMap::from([
            ("KJFK".to_string(), Airport::at(40.64, -73.78)),
            ("KDCA".to_string(), Airport::at(38.85, -77.04)),
        ])
    }

    fn plan(route: &str) -> Plan<'_> {
        Plan {
            dep: "KJFK",
            arr: "KDCA",
            route,
            cruise_ft: 24_000.0,
            filed_tas: 440.0,
            arr_ll: [38.85, -77.04],
        }
    }

    /// AC4: the backtest's TOD path and the live arrival predictor must time the same aircraft on
    /// the same route with the same wind. Timed all the way to the field, `eta_at_along` must equal
    /// `predict::arrival_eta` to the second, on a route that resolves (`RBV WHITE SIE` through the
    /// bundled nav data) and on one that doesn't (great-circle fallback). Resolving the route
    /// differently, sampling the wind at another altitude, or capping the TAS another way breaks it.
    #[test]
    fn tod_eta_matches_arrival_eta_at_the_field() {
        let nav = NavData::load();
        let ap = airports();
        let winds = layered_winds();
        let profile = AircraftProfile::default();
        for route in ["RBV WHITE SIE", ""] {
            let p = plan(route);
            let snap = pos(0, 40.2, -74.0, 18_000, 380);
            let ac = arrival_input(&p, &snap, &profile);
            let live = predict::arrival_eta(&nav, &ap, &winds, &profile, &ac, 0.0, t0());
            let ours = eta_at_along(&nav, &ap, &winds, &profile, &ac, live.route_nm, t0());
            assert_eq!(ours, live.eta, "route {route:?}");
            // Positive control: a point short of the field is earlier.
            let short = eta_at_along(&nav, &ap, &winds, &profile, &ac, live.route_nm - 50.0, t0());
            assert!(short < live.eta);
        }
    }

    /// `predict_case`'s arrival is `predict::arrival_eta`'s, not a recomputation.
    #[test]
    fn the_scored_arrival_is_the_live_arrival_eta() {
        let nav = NavData::load();
        let ap = airports();
        let winds = layered_winds();
        let profile = AircraftProfile::default();
        let p = plan("RBV WHITE SIE");
        let obs = Observed {
            snapshot: pos(0, 40.2, -74.0, 18_000, 380),
            tod: pos(1_200, 39.4, -75.6, 24_000, 450),
            touchdown: t0() + Duration::seconds(2_400),
        };
        let pred = predict_case(&nav, &ap, &winds, &profile, &p, &obs);
        let ac = arrival_input(&p, &obs.snapshot, &profile);
        let live = predict::arrival_eta(&nav, &ap, &winds, &profile, &ac, 0.0, t0());
        assert_eq!(pred.arrival, live.eta);
        assert!(pred.tod > t0() && pred.tod < pred.arrival);
    }

    /// A full flight: roll at 15 kt steps, climb, cruise at 24,000 ft, descend, land near KDCA.
    fn flight() -> Vec<HistPosition> {
        let mut v = vec![
            pos(0, 40.64, -73.78, 20, 10),
            pos(15, 40.64, -73.78, 20, 120),
        ];
        v.push(pos(30, 40.6, -73.8, 800, 160)); // under 1,000 ft above the roll: not yet
        v.push(pos(45, 40.55, -73.85, 1_500, 200)); // snapshot
        v.push(pos(600, 40.0, -74.8, 24_000, 450));
        v.push(pos(1_200, 39.5, -75.8, 24_100, 450)); // TOD (within the cruise band of the top)
        v.push(pos(1_260, 39.45, -75.9, 22_500, 430));
        v.push(pos(2_000, 38.9, -76.95, 1_000, 150));
        v.push(pos(2_030, 38.86, -77.03, 30, 60)); // last airborne tick
        v.push(pos(2_045, 38.85, -77.04, 20, 30)); // landing roll
        v
    }

    #[test]
    fn observe_finds_snapshot_tod_and_touchdown() {
        let o = observe(&flight(), [38.85, -77.04]).expect("scorable");
        assert_eq!(o.snapshot.altitude, 1_500);
        assert_eq!(o.tod.altitude, 24_100);
        assert_eq!(o.touchdown, t0() + Duration::seconds(2_030));
    }

    #[test]
    fn observe_refuses_unscorable_flights() {
        // Landed somewhere else.
        assert!(observe(&flight(), [40.64, -73.78]).is_none());
        // Never landed (disconnected airborne).
        let mut v = flight();
        v.truncate(8);
        assert!(observe(&v, [38.85, -77.04]).is_none());
        // Descent start can't be timed: the fix after the TOD is four minutes later.
        let mut v = flight();
        v[6].ts = v[5].ts + Duration::seconds(240);
        assert!(observe(&v, [38.85, -77.04]).is_none());
        // Positive control: the untouched flight is scorable.
        assert!(observe(&flight(), [38.85, -77.04]).is_some());
    }

    fn row(kind: &str, key: &str, desc_fpm: f64) -> crate::models::AircraftProfileBody {
        crate::models::AircraftProfileBody {
            kind: kind.into(),
            key: key.into(),
            name: key.into(),
            climb_ias_lo: 250.0,
            climb_ias_hi: 290.0,
            climb_mach: None,
            climb_fpm_lo: 2_000.0,
            climb_fpm_hi: 1_500.0,
            cruise_tas: None,
            cruise_mach: None,
            service_ceiling_ft: 41_000.0,
            desc_mach: None,
            desc_ias_hi: 290.0,
            desc_ias_lo: 250.0,
            desc_fpm,
            updated_at: t0(),
            updated_by: None,
        }
    }

    /// Set B replaces only the types the builder profiled, starting from what set A resolved them
    /// to; everything else resolves exactly as in set A.
    #[test]
    fn set_b_overlays_only_profiled_types() {
        use super::super::samples::Field;
        use super::super::stats::{Accumulator, Source};
        let rows = vec![
            row("default", "default", 1_800.0),
            row("wake", "H", 2_200.0),
            row("type", "B738", 1_900.0),
        ];
        let mut acc = Accumulator::default();
        for _ in 0..20 {
            acc.add_flight(
                "B77W",
                "H",
                &[
                    (Field::ClimbFpmLo, 1_700.0),
                    (Field::CruiseTas, 480.0),
                    (Field::DescFpm, 2_600.0),
                ],
            );
            acc.add_flight("E75L", "", &[(Field::DescFpm, 2_100.0)]);
        }
        let src = Source {
            from: "a".into(),
            to: "b".into(),
            flights_read: 40,
            training_flights: 40,
            held_out_flights: 0,
            held_out_rule: "t".into(),
        };
        let built = acc.finish(src, |_| None);
        let (a, b, changed) = profile_sets(&rows, &built);
        assert_eq!(
            changed.into_iter().collect::<Vec<_>>(),
            vec!["B77W".to_string()]
        );
        assert_eq!(
            a.resolve("B77W", "H").desc_fpm,
            2_200.0,
            "A: the heavy wake row"
        );
        let b77w = b.resolve("B77W", "H");
        assert_eq!(b77w.desc_fpm, 2_600.0);
        assert_eq!(b77w.climb_fpm_lo, 1_700.0);
        assert_eq!(
            b77w.service_ceiling_ft, 41_000.0,
            "unmeasured fields come from A's resolution"
        );
        // E75L has no climb or cruise measurement, so it is not profiled and resolves as in A.
        assert_eq!(b.resolve("E75L", ""), a.resolve("E75L", ""));
        assert_eq!(b.resolve("B738", ""), a.resolve("B738", ""));
    }

    #[test]
    fn error_stats_are_exact() {
        let s = ErrorStats::of(&[60.0, -30.0, 90.0, -120.0]);
        assert_eq!(s.n, 4);
        assert_eq!(s.mae, 75.0);
        assert_eq!(s.bias, 0.0);
        // abs sorted 30, 60, 90, 120: p90 at rank 2.7 → 90 + 0.7·30 = 111.
        assert!((s.p90_abs - 111.0).abs() < 1e-9);
    }

    // ---- #828: the backtest times on `predict::arrival_route` and refuses stale winds ----------
    //
    /// 80 kt from 225° (on the nose of a south-westbound KJFK→KDCA flight) at every level over
    /// 37–42°N, 79–72°W.
    fn headwind_field() -> Winds {
        Winds::test_grid(
            (37.0, -79.0),
            (6, 8),
            &[3_000.0, 18_000.0, 30_000.0],
            t0(),
            |_, _, _| (-57, -57, -20),
        )
    }

    /// Under a per-segment RAP field, on a route that resolves and on one that does not, the
    /// backtest's own timing to the field is `predict::arrival_eta`'s to the second. Positive
    /// control: the field moves the ETA by more than 30 s against still air.
    #[test]
    fn tod_eta_matches_arrival_eta_at_the_field_under_a_sheared_rap_field() {
        let nav = NavData::load();
        let ap = airports();
        let field = Winds::test_shear();
        let profile = AircraftProfile::default();
        for route in ["RBV WHITE SIE", ""] {
            let p = plan(route);
            let ac = arrival_input(&p, &pos(0, 40.2, -74.0, 31_000, 450), &profile);
            let live = predict::arrival_eta(&nav, &ap, &field, &profile, &ac, 0.0, t0());
            let ours = eta_at_along(&nav, &ap, &field, &profile, &ac, live.route_nm, t0());
            assert_eq!(ours, live.eta, "route {route:?}");
            let still =
                predict::arrival_eta(&nav, &ap, &Winds::default(), &profile, &ac, 0.0, t0());
            assert!(
                (live.eta - still.eta).num_seconds().abs() > 30,
                "route {route:?}"
            );
        }
    }

    /// From the ground (a branch the backtest never replays, since it starts airborne), the
    /// backtest's timing still equals `arrival_eta`'s: same route, same ground handling, no ground
    /// allowance on either side.
    #[test]
    fn tod_eta_matches_arrival_eta_from_the_ground() {
        let nav = NavData::default();
        let ap = airports();
        let field = headwind_field();
        let profile = AircraftProfile::default();
        let p = plan("");
        let ac = arrival_input(&p, &pos(0, 40.64, -73.78, 20, 0), &profile);
        let live = predict::arrival_eta(&nav, &ap, &field, &profile, &ac, 0.0, t0());
        let ours = eta_at_along(&nav, &ap, &field, &profile, &ac, live.route_nm, t0());
        assert_eq!(ours, live.eta);
    }

    /// The backtest resolves a route only through `predict::arrival_route`: no private copy of the
    /// resolution (AC4, "not a copy of the formula"). A source scan, because the property is an
    /// absence that no call can show.
    #[test]
    fn the_backtest_resolves_routes_only_through_predict_arrival_route() {
        let src = include_str!("backtest.rs");
        let prod = &src[..src.find("#[cfg(test)]").expect("the test module marker")];
        let code: String = prod
            .lines()
            .map(|l| l.split("//").next().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(code.contains("predict::arrival_route("), "anchor missing");
        for copy in ["route_path(", "path_len_nm(", "GROUND_ROUTE_FACTOR"] {
            assert!(
                !code.contains(copy),
                "backtest.rs resolves routes itself: {copy}"
            );
        }
    }

    /// One stored flight: its row (`route` filed on the flight) and its positions.
    async fn seed(pool: &PgPool, session_id: i64, route: &str, positions: &[HistPosition]) {
        let (first, last) = (positions[0].ts, positions[positions.len() - 1].ts);
        sqlx::query(
            "insert into stats.flight (session_id, cid, callsign, logon_time, first_seen, \
                last_seen, status, departure, arrival, aircraft_short, aircraft_faa, cruise_tas, \
                cruise_alt, route) \
             values ($1, 1, 'TEST', $2, $2, $3, 'completed', 'KJFK', 'KDCA', 'B738', 'B738/L', \
                440, 24000, $4)",
        )
        .bind(session_id)
        .bind(first)
        .bind(last)
        .bind(route)
        .execute(pool)
        .await
        .unwrap();
        for p in positions {
            sqlx::query(
                "insert into stats.position (session_id, ts, lat, lon, altitude, groundspeed, \
                    heading) values ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(session_id)
            .bind(p.ts)
            .bind(p.lat)
            .bind(p.lon)
            .bind(p.altitude)
            .bind(p.groundspeed)
            .bind(p.heading)
            .execute(pool)
            .await
            .unwrap();
        }
    }

    async fn seed_winds(pool: &PgPool, ts: DateTime<Utc>, w: &Winds) {
        sqlx::query("insert into stats.winds (ts, data) values ($1, $2)")
            .bind(ts)
            .bind(sqlx::types::Json(w))
            .execute(pool)
            .await
            .unwrap();
    }

    /// `backtest::run` over the window around [`flight`], with set A and set B both the default
    /// profile (so `a` and `b` agree), on `nav`.
    async fn run_all(pool: &PgPool, nav: &NavData) -> (Vec<Scored>, usize) {
        let ap = airports();
        let table = ProfileTable::new(AircraftProfile::default(), HashMap::new(), HashMap::new());
        let none = BTreeSet::new();
        let replay = Replay {
            nav,
            airports: &ap,
            set_a: &table,
            set_b: &table,
            b_types: &none,
        };
        run(
            pool,
            &replay,
            t0() - Duration::hours(1),
            t0() + Duration::hours(3),
        )
        .await
        .unwrap()
    }

    /// The arrival error `predict_case` gives `positions` under `winds` and `route`.
    fn expected(nav: &NavData, winds: &Winds, route: &str, positions: &[HistPosition]) -> f64 {
        let obs = observe(positions, [38.85, -77.04]).unwrap();
        let mut p = plan(route);
        p.cruise_ft = 24_000.0;
        let pred = predict_case(
            nav,
            &airports(),
            winds,
            &AircraftProfile::default(),
            &p,
            &obs,
        );
        (pred.arrival - obs.touchdown).num_milliseconds() as f64 / 1000.0
    }

    /// The backtest applies training's winds freshness rule: a snapshot 130 min before the replay
    /// instant is ignored (still air), one 110 min before it is flown.
    #[sqlx::test]
    async fn the_backtest_flies_still_air_past_a_stale_winds_snapshot_and_the_field_within_two_hours(
        pool: PgPool,
    ) {
        let nav = NavData::default();
        let f = flight();
        seed(&pool, 5, "", &f).await;
        let snap = observe(&f, [38.85, -77.04]).unwrap().snapshot.ts;
        let field = headwind_field();
        seed_winds(&pool, snap - Duration::minutes(130), &field).await;
        let (stale, _) = run_all(&pool, &nav).await;
        assert_eq!(
            stale[0].a.arrival_sec,
            expected(&nav, &Winds::default(), "", &f)
        );

        seed_winds(&pool, snap - Duration::minutes(110), &field).await;
        let (fresh, _) = run_all(&pool, &nav).await;
        assert_eq!(fresh[0].a.arrival_sec, expected(&nav, &field, "", &f));
        assert!((fresh[0].a.arrival_sec - stale[0].a.arrival_sec).abs() > 30.0);
    }

    /// Each held-out flight replays on the plan revision in force at its own replay instant (a
    /// revision filed later is not used), and a flight with no revision on its `stats.flight` route.
    #[sqlx::test]
    async fn each_held_out_flight_is_replayed_on_the_plan_revision_in_force_at_its_own_snapshot(
        pool: PgPool,
    ) {
        let nav = NavData::load();
        let f = flight();
        let snap = observe(&f, [38.85, -77.04]).unwrap().snapshot.ts;
        seed(&pool, 5, "", &f).await;
        let later: Vec<_> = f
            .iter()
            .map(|p| HistPosition {
                ts: p.ts + Duration::seconds(7),
                ..p.clone()
            })
            .collect();
        seed(&pool, 10, "RBV WHITE SIE", &later).await;
        for (at, route) in [
            (snap - Duration::minutes(5), "RBV WHITE SIE"),
            (snap + Duration::minutes(5), ""),
        ] {
            sqlx::query(
                "insert into stats.flight_plan (session_id, effective_from, route, cruise_alt) \
                 values (5, $1, $2, 24000)",
            )
            .bind(at)
            .bind(route)
            .execute(&pool)
            .await
            .unwrap();
        }
        let (scored, held) = run_all(&pool, &nav).await;
        assert_eq!((held, scored.len()), (2, 2));
        let resolved = expected(&nav, &Winds::default(), "RBV WHITE SIE", &f);
        assert_ne!(
            resolved,
            expected(&nav, &Winds::default(), "", &f),
            "routes must differ"
        );
        assert_eq!(
            scored[0].a.arrival_sec, resolved,
            "flight 5: the revision in force"
        );
        assert_eq!(
            scored[1].a.arrival_sec,
            expected(&nav, &Winds::default(), "RBV WHITE SIE", &later),
            "flight 10: no revision, the flight row's route"
        );
    }

    /// Two held-out flights whose positions interleave in time, beside a training flight: each is
    /// scored on its own track, and the training flight is not scored.
    #[sqlx::test]
    async fn run_scores_each_held_out_flight_on_its_own_positions(pool: PgPool) {
        let nav = NavData::default();
        let five = flight();
        let mut ten: Vec<_> = flight()
            .into_iter()
            .map(|p| HistPosition {
                ts: p.ts + Duration::seconds(7),
                ..p
            })
            .collect();
        ten[3].altitude = 2_500;
        ten[4].ts += Duration::seconds(300);
        seed(&pool, 5, "", &five).await;
        seed(&pool, 10, "", &ten).await;
        seed(&pool, 1, "", &flight()).await;
        let (scored, held) = run_all(&pool, &nav).await;
        assert_eq!(held, 2);
        assert_eq!(scored.len(), 2);
        let (e5, e10) = (
            expected(&nav, &Winds::default(), "", &five),
            expected(&nav, &Winds::default(), "", &ten),
        );
        assert_ne!(e5, e10, "the two tracks must score differently");
        assert_eq!(
            [scored[0].a.arrival_sec, scored[1].a.arrival_sec],
            [e5, e10]
        );
    }

    /// The report's rows, worked by hand from two flights: A errs (+60 TOD, -120 arrival) and
    /// (-30, +90); B errs (+10, -20) and (+30, -40); only the second flight's type is in B.
    #[test]
    fn the_report_gives_mae_bias_and_p90_for_both_sets_and_both_points() {
        let e = |tod_sec, arrival_sec| CaseError {
            tod_sec,
            arrival_sec,
        };
        let scored = vec![
            Scored {
                ty: "B738".into(),
                in_b: false,
                a: e(60.0, -120.0),
                b: e(10.0, -20.0),
            },
            Scored {
                ty: "B77W".into(),
                in_b: true,
                a: e(-30.0, 90.0),
                b: e(30.0, -40.0),
            },
        ];
        let r = report(&scored, 5, "A", "B");
        assert!(r.contains("5 held-out flights, 2 scored"), "{r}");
        assert!(r.contains("## All scored flights (2 flights)"), "{r}");
        assert!(
            r.contains("## Flights of types the new set profiles (1 flights)"),
            "{r}"
        );
        // All flights. A TOD: |60|,|30| → MAE 45, bias +15, p90 = 30 + 0.9·30 = 57.
        for row in [
            "| A | TOD | 45 | +15 | 57 |",
            "| A | arrival | 105 | -15 | 117 |",
            "| B | TOD | 20 | +20 | 28 |",
            "| B | arrival | 30 | -30 | 38 |",
            // The B-typed flight alone.
            "| A | TOD | 30 | -30 | 30 |",
            "| B | arrival | 40 | -40 | 40 |",
        ] {
            assert!(r.contains(row), "missing {row}\n{r}");
        }
    }
}
