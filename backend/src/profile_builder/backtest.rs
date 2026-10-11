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

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use super::{is_held_out, samples::CRUISE_BAND_FT, samples::MAX_RATE_GAP_SEC, type_and_wake};
use crate::{
    errors::ApiError,
    feed::{
        airports::AirportDb,
        fca,
        flow::gc_dist,
        nav::NavData,
        predict::{self, ArrivalInput},
        trajectory::{self, AircraftProfile, ProfileTable},
        winds::Winds,
    },
    repos::{
        profile_history::{self, HistFlight, HistPosition},
        stats as stats_repo,
    },
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

/// The route length (nm) and route headwind `arrival_eta` times an aircraft against: the resolved
/// filed route when it resolves, else the great circle to the field. The same two public calls
/// `arrival_eta` makes, so a TOD timed on them sits on the route the arrival ETA was timed on;
/// `tod_eta_matches_arrival_eta_at_the_field` fails the moment the two disagree.
fn route_context(
    nav: &NavData,
    airports: &AirportDb,
    winds: &Winds,
    ac: &ArrivalInput,
) -> (f64, Option<f64>) {
    let [lat, lon] = ac.pos;
    match fca::route_path(
        nav, airports, ac.dep, ac.arr, ac.route, lat, lon, ac.hdg, ac.gs,
    ) {
        Some(path) => (
            predict::path_len_nm(&path),
            winds.route_headwind(&path, ac.cruise_ft),
        ),
        None => (
            gc_dist(lat, lon, ac.arr_ll[0], ac.arr_ll[1]),
            winds.route_headwind(&[ac.pos, ac.arr_ll], ac.cruise_ft),
        ),
    }
}

/// The ETA `along_nm` ahead of the aircraft on the route `arrival_eta` would time it on.
pub fn eta_at_along(
    nav: &NavData,
    airports: &AirportDb,
    winds: &Winds,
    profile: &AircraftProfile,
    ac: &ArrivalInput,
    along_nm: f64,
    now: DateTime<Utc>,
) -> DateTime<Utc> {
    let (route_nm, headwind) = route_context(nav, airports, winds, ac);
    predict::eta_along_route(
        ac.gs >= super::samples::AIRBORNE_GS_KT as i64,
        route_nm,
        along_nm.clamp(0.0, route_nm),
        ac.alt_ft,
        ac.gs as f64,
        ac.cruise_ft,
        ac.cruise_tas,
        airports,
        ac.arr,
        profile,
        headwind,
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
    let (here_nm, _) = route_context(nav, airports, winds, &ac);
    let at_tod = arrival_input(plan, &obs.tod, profile);
    let (tod_nm, _) = route_context(nav, airports, winds, &at_tod);
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

/// Replay the held-out flights wholly inside `[from, to)` under both sets.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    pool: &PgPool,
    nav: &NavData,
    airports: &AirportDb,
    set_a: &ProfileTable,
    set_b: &ProfileTable,
    b_types: &std::collections::BTreeSet<String>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<(Vec<Scored>, usize), ApiError> {
    let flights = profile_history::flights(pool, from, to).await?;
    let mut scored = Vec::new();
    let mut held_out = 0;
    for f in flights.iter().filter(|f| is_held_out(f.session_id)) {
        held_out += 1;
        if let Some(s) = score_flight(pool, nav, airports, set_a, set_b, b_types, f).await? {
            scored.push(s);
        }
    }
    Ok((scored, held_out))
}

async fn score_flight(
    pool: &PgPool,
    nav: &NavData,
    airports: &AirportDb,
    set_a: &ProfileTable,
    set_b: &ProfileTable,
    b_types: &std::collections::BTreeSet<String>,
    f: &HistFlight,
) -> Result<Option<Scored>, ApiError> {
    let Some(arr) = airports.get(&f.arrival.to_ascii_uppercase()) else {
        return Ok(None);
    };
    let positions = profile_history::positions(pool, f.session_id).await?;
    let Some(obs) = observe(&positions, [arr.lat, arr.lon]) else {
        return Ok(None);
    };
    // The plan in force at the replay instant (revisions are kept in stats.flight_plan); TAS has no
    // revision history, so it comes from the flight row.
    let rev = profile_history::plan_at(pool, f.session_id, obs.snapshot.ts).await?;
    let route = rev
        .as_ref()
        .and_then(|r| r.route.clone())
        .or_else(|| f.route.clone())
        .unwrap_or_default();
    let cruise_alt = rev.as_ref().and_then(|r| r.cruise_alt).or(f.cruise_alt);
    let plan = Plan {
        dep: &f.departure,
        arr: &f.arrival,
        route: &route,
        cruise_ft: trajectory::parse_alt_ft(&cruise_alt.map(|a| a.to_string()).unwrap_or_default()),
        filed_tas: f.cruise_tas.unwrap_or(0) as f64,
        arr_ll: [arr.lat, arr.lon],
    };
    let winds = stats_repo::winds_at(pool, obs.snapshot.ts)
        .await?
        .unwrap_or_default();
    let (ty, wake) = type_and_wake(f);
    let err = |profile: &AircraftProfile| {
        let p = predict_case(nav, airports, &winds, profile, &plan, &obs);
        CaseError {
            tod_sec: (p.tod - obs.tod.ts).num_milliseconds() as f64 / 1000.0,
            arrival_sec: (p.arrival - obs.touchdown).num_milliseconds() as f64 / 1000.0,
        }
    };
    Ok(Some(Scored {
        in_b: b_types.contains(&ty),
        a: err(set_a.resolve(&ty, &wake)),
        b: err(set_b.resolve(&ty, &wake)),
        ty,
    }))
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
}
