//! The single arrival/crossing prediction service. Every surface that estimates when a connected
//! aircraft reaches a point — the arrival ladder, airport-flow demand, runway ETE, and FCA
//! metering — resolves the filed route **once** here and times position-along-route through the
//! one [`trajectory::VerticalProfile`] model. That keeps a given aircraft's ETA identical on every
//! surface and consistent with the real route drawn on the map (issue #63): before this, the
//! arrival surfaces timed aircraft on a straight-line great circle to the field while FCA metering
//! (correctly) used the real, longer, dog-legged route.
//!
//! Route resolution ([`fca::route_path`] → the nav engine) is the same one FCA metering uses. When
//! a route can't be resolved past its dep/arr endpoints (blank/`DCT` route, fixes the nav engine
//! doesn't know, unknown airport), we fall back to a straight-line great circle to the field so an
//! arrival is never dropped — and for a **not-yet-airborne** flight that fallback carries the
//! [`GROUND_ROUTE_FACTOR`] allowance, because issue #63 only meant to drop the historical padding
//! where an actual enroute path *is* resolved, not to start timing `DCT` prefiles on a distance no
//! real routing can fly (that fed metered wheels-up / CFR times early).

use std::cell::OnceCell;
use std::collections::HashMap;
use std::sync::LazyLock;

use chrono::{DateTime, Duration, Utc};

use super::airports::{AirportDb, field_elevation_ft};
use super::fca;
use super::flow::gc_dist;
use super::nav::NavData;
use super::trajectory::{self, AircraftProfile, ProfileTable, RouteWind};
use super::vatsim::{FlightPlan, Pilot, Prefile};
use super::winds::Winds;

/// Minutes from a ground flight's filed departure time, a pushback, to its proposed wheels-up when it
/// holds no scheduler time (no ready time, release, CFR or GDP EDCT). vTBFM draws an unscheduled
/// departure at the same point: its filed time plus 10 minutes of assumed taxi (vTBFM manual §18.2,
/// `zla-artcc/vTBFM` `906db6ca`) (#814).
pub const PROPOSED_WHEELS_UP_MIN: i64 = 10;

/// Great-circle → flyable-route length multiplier for a ground/proposed flight whose filed route
/// won't resolve to an enroute path. Airway routings run a little longer than the direct great
/// circle; without this a bare `DCT`/blank filing would be timed on an unachievable distance and
/// its metered wheels-up / CFR time would skew early (#63).
pub const GROUND_ROUTE_FACTOR: f64 = 1.12;

/// Total great-circle length (nm) of a resolved route path.
pub fn path_len_nm(path: &[[f64; 2]]) -> f64 {
    path.windows(2)
        .map(|w| gc_dist(w[0][0], w[0][1], w[1][0], w[1][1]))
        .sum()
}

/// An ETA to a point along the route, and the groundspeed the profile predicts **at that point**.
///
/// Both come from the same [`trajectory::VerticalProfile`], so they always describe the same
/// aircraft in the same phase of flight. They are returned together deliberately: MIT spacing is a
/// *distance* at the crossing fix, and sizing that gap with cruise speed under-provisions every
/// aircraft crossing on descent (#355 — a 20 MIT flow over a low arrival fix realized ~12 nm).
pub struct AlongRouteEta {
    pub eta: DateTime<Utc>,
    /// Predicted groundspeed (kt) at the target point — the phase-appropriate (descent/cruise/climb)
    /// TAS with wind and any observed-GS anchoring applied, via
    /// [`trajectory::VerticalProfile::ground_speed_at`]. This is the speed the paired `eta` was
    /// integrated against, not the filed cruise speed.
    pub gs_kt: f64,
    /// The profile both were read from, for a caller that explains them (the FCA debug view).
    pub profile: trajectory::VerticalProfile,
}

/// ETA to a point `along_nm` along the route ahead of the aircraft's current position, timed with
/// the shared vertical-profile + winds model, together with the predicted groundspeed there. The
/// profile is built over the whole remaining route to the destination (so descent is modeled when
/// the target is near the field); the target sits
/// `along_nm` ahead. Airborne aircraft start from their current altitude; ground aircraft climb
/// from the surface, and `now` is their wheels-up. The descent ends at `arr_icao`'s field elevation,
/// resolved from `airports` by [`profile_from_here`] rather than passed in (#411). An airborne aircraft's
/// prediction is anchored to `observed_gs_kt` when it is established at cruise
/// ([`trajectory::VerticalProfile::anchor_to_observed_gs`]).
#[allow(clippy::too_many_arguments)]
pub fn eta_along_route(
    airborne: bool,
    route_len_nm: f64,
    along_nm: f64,
    cur_alt_ft: f64,
    observed_gs_kt: f64,
    cruise_alt_ft: f64,
    cruise_tas: f64,
    airports: &AirportDb,
    arr_icao: &str,
    profile: &AircraftProfile,
    wind: RouteWind,
    now: DateTime<Utc>,
) -> AlongRouteEta {
    let vp = profile_from_here(
        airborne,
        route_len_nm,
        cur_alt_ft,
        observed_gs_kt,
        cruise_alt_ft,
        cruise_tas,
        airports,
        arr_icao,
        profile,
        wind,
    );
    // Distances are nm-to-destination: the aircraft is at `route_len_nm`, the target `along_nm`
    // ahead of it (i.e. `route_len_nm − along_nm` from the field).
    let target_d = (route_len_nm - along_nm).max(0.0);
    let sec = vp.time_between(route_len_nm, target_d);
    AlongRouteEta {
        eta: now + Duration::seconds(sec as i64),
        gs_kt: vp.ground_speed_at(target_d),
        profile: vp,
    }
}

/// The vertical profile [`eta_along_route`] and [`Trajectory`] share: airborne aircraft
/// start from their current altitude, anchored to their observed groundspeed; ground aircraft climb
/// from the surface on the raw profile.
///
/// This is also the **one** place the descent's end altitude is resolved: it looks the arrival field
/// elevation up from `airports` itself instead of taking it as an argument, so no caller can pass the
/// wrong one — or drop it and silently get a sea-level descent, which five separate call sites could
/// each do before #411. [`trajectory::VerticalProfile::build`] keeps the raw-`f64` entry point for the
/// model's own tests, which is what keeps `feed::trajectory` free of any airport-db dependency.
#[allow(clippy::too_many_arguments)]
pub fn profile_from_here(
    airborne: bool,
    route_len_nm: f64,
    cur_alt_ft: f64,
    observed_gs_kt: f64,
    cruise_alt_ft: f64,
    cruise_tas: f64,
    airports: &AirportDb,
    arr_icao: &str,
    profile: &AircraftProfile,
    wind: RouteWind,
) -> trajectory::VerticalProfile {
    let start_alt = if airborne { cur_alt_ft } else { 0.0 };
    let vp = trajectory::VerticalProfile::build(
        start_alt,
        route_len_nm,
        field_elevation_ft(airports, arr_icao),
        cruise_alt_ft,
        cruise_tas,
        profile,
        wind,
    );
    if airborne {
        vp.anchor_to_observed_gs(observed_gs_kt)
    } else {
        vp
    }
}

/// ETA to the destination field + the along-route distance still to fly.
pub struct ArrivalPrediction {
    pub eta: DateTime<Utc>,
    /// Along-route distance to the field, nm — from the resolved filed route, or a straight-line
    /// great circle when the route can't be resolved.
    pub route_nm: f64,
}

/// One aircraft's state for an arrival prediction. `pos` is the current position for an airborne
/// aircraft, or the departure field for a ground/proposed one; `arr_ll` is the destination field
/// (the straight-line fallback target). `cruise_ft` / `cruise_tas` are the filed cruise altitude
/// and the profile-capped cruise true airspeed.
pub struct ArrivalInput<'a> {
    pub dep: &'a str,
    pub arr: &'a str,
    pub route: &'a str,
    pub pos: [f64; 2],
    pub alt_ft: f64,
    pub gs: i64,
    pub hdg: i64,
    pub arr_ll: [f64; 2],
    pub cruise_ft: f64,
    pub cruise_tas: f64,
}

/// The route an arrival is timed along: the resolved path, its length, and whether the aircraft is
/// airborne ([`is_airborne`]).
pub struct ArrivalRoute {
    pub path: Vec<[f64; 2]>,
    /// Along-route distance to the field, nm. A ground flight whose route does not resolve past its
    /// endpoints carries [`GROUND_ROUTE_FACTOR`].
    pub route_nm: f64,
    pub airborne: bool,
}

/// Resolve the route [`arrival_eta`] times an aircraft along: the filed route via
/// [`fca::route_path`] (the trimmed remaining route for airborne aircraft, the full dep→arr route
/// for ground/proposed), else a straight-line great circle `pos → arr_ll`, through [`resolve_path`]
/// and [`route_scale`] as [`trajectory`] resolves it, except that a landed pilot still gets the
/// great circle here. Public so the offline profile backtest (`profile_builder::backtest`) times a
/// point short of the field on exactly this route.
pub fn arrival_route(nav: &NavData, airports: &AirportDb, ac: &ArrivalInput) -> ArrivalRoute {
    let airborne = is_airborne(ac.gs);
    let (path, resolved) = resolve_path(
        nav,
        airports,
        (ac.dep, ac.arr, ac.route),
        (ac.pos, ac.hdg, ac.gs),
        Some(ac.arr_ll),
        false,
    )
    .unwrap_or_else(|| (vec![ac.pos, ac.arr_ll], false));
    let route_nm = path_len_nm(&path) * route_scale(airborne, resolved);
    ArrivalRoute {
        path,
        route_nm,
        airborne,
    }
}

/// Predict when an aircraft reaches its destination field, and how far along its route that is.
///
/// Times the route [`arrival_route`] resolves through [`eta_along_route`] with the target at the
/// field; a ground aircraft is wheels-up at `now`. No surface calls this: they all go through
/// [`trajectory`], which decides the inputs this takes as arguments. The offline profile backtest
/// (`profile_builder::backtest`) does.
pub fn arrival_eta(
    nav: &NavData,
    airports: &AirportDb,
    winds: &Winds,
    profile: &AircraftProfile,
    ac: &ArrivalInput,
    now: DateTime<Utc>,
) -> ArrivalPrediction {
    let ArrivalRoute {
        path,
        route_nm,
        airborne,
    } = arrival_route(nav, airports, ac);
    let wind = winds.along(&path, route_nm);
    let eta = eta_along_route(
        airborne,
        route_nm,
        route_nm,
        ac.alt_ft,
        ac.gs as f64,
        ac.cruise_ft,
        ac.cruise_tas,
        airports,
        ac.arr,
        profile,
        wind,
        now,
    )
    .eta;
    ArrivalPrediction { eta, route_nm }
}

// ---------------------------------------------------------------------------------------------
// The one entry point (#813).
//
// Every surface that times a flight (airport flow, GDP and CFR, runway ETE, sector demand, FCA
// metering and IDST, the projected map, the per-fix table) calls [`trajectory`] and reads its
// times off the [`Trajectory`] it returns. The flight's inputs are decided here and nowhere else:
//
// - **Airborne**: `gs >= AIRBORNE_GS_KT`, no altitude term. It is the threshold the route
//   resolver trims on, so a caller using another value labels a flight one way and times it the
//   other; an `alt > 300` term reads an MSL altitude and means nothing at a high field.
// - **Route**: the filed route resolved from where the flight is (`fca::route_path`), else a great
//   circle to the field, except for a pilot that has arrived (`ARRIVED_NM`); a ground or prefiled
//   flight with neither is timed over `NOMINAL_ROUTE_NM` in still air.
// - **Padding**: `GROUND_ROUTE_FACTOR` for every not-yet-airborne flight whose route does not
//   resolve past its endpoints, so no surface times a `DCT` filing on a distance no routing flies.
// - **Filed altitude** (`trajectory::filed_altitude_ft`, unknown timed at 35,000 ft) and **filed
//   cruise speed** (`filed_tas`, capped by the profile through `capped_cruise_tas`).
// - **Wheels-up**: now for an airborne flight; the released wheels-up (a ready time, FCA release,
//   CFR or GDP EDCT) for a ground flight that holds one; otherwise, for a connected pilot, the
//   proposed wheels-up, its filed `deptime` plus `PROPOSED_WHEELS_UP_MIN` (`proposed_wheels_up`). A
//   prefile gets no proposed wheels-up: it is timed only from a lock it holds. No taxi allowance is
//   added anywhere: a ready time is a runway time and the trajectory is flown from wheels-up (#814).
// - **Wind**: each point's own altitude along the profile (`Winds::along`, #836).
// ---------------------------------------------------------------------------------------------

/// Groundspeed (kt) from which every surface treats a flight as airborne (#813).
pub const AIRBORNE_GS_KT: i64 = 50;

/// Whether a flight at `gs_kt` is airborne. The one test every surface times and labels with.
pub fn is_airborne(gs_kt: i64) -> bool {
    gs_kt >= AIRBORNE_GS_KT
}

/// Floor (kt) on a predicted crossing speed before a miles-in-trail is turned into a time gap, so a
/// slow profile sample cannot open an hour-long gap.
pub const MIN_CROSS_SPEED_KT: f64 = 120.0;

/// The leg (nm) a ground or prefiled flight with no route and no field position is timed over.
const NOMINAL_ROUTE_NM: f64 = 300.0;

/// The wheels-up each flight is held to, by callsign: the latest of its issued CFR, its releases in
/// live, visible FCAs and its published GDP slot (`repos::flow::locked_wheels_up`).
pub type LockedWheelsUp = HashMap<String, DateTime<Utc>>;

/// No locks: for a caller that times airborne flights only, or passes each released wheels-up itself
/// (sector demand), and for the historical replay, which holds none of today's locks.
pub static NO_LOCKS: LazyLock<LockedWheelsUp> = LazyLock::new(HashMap::new);

/// Everything [`trajectory`] reads besides the flight.
pub struct Inputs<'a> {
    pub nav: &'a NavData,
    pub airports: &'a AirportDb,
    pub winds: &'a Winds,
    pub profiles: &'a ProfileTable,
    /// The wheels-up each flight holds. A surface passes a flight's entry to [`trajectory`] as its
    /// `released`, so a locked flight is timed from its lock and never from the proposed wheels-up
    /// (#814).
    pub locked: &'a LockedWheelsUp,
}

impl Inputs<'_> {
    /// The wheels-up `callsign` is held to, if any.
    pub fn locked_wheels_up(&self, callsign: &str) -> Option<DateTime<Utc>> {
        self.locked.get(callsign).copied()
    }
}

/// The flight to time: a connected pilot, or a prefile (no position; timed from its departure).
#[derive(Clone, Copy)]
pub enum Flight<'a> {
    Pilot(&'a Pilot),
    Prefile(&'a Prefile),
}

/// Where a flight is timed from: its live position, or its departure field for a prefile.
#[derive(Clone, Copy)]
struct Origin {
    pos: [f64; 2],
    hdg: i64,
    gs: i64,
    alt_ft: f64,
}

/// A point a [`Trajectory`] has reached after some flying time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progress {
    /// Distance along [`Trajectory::path`] from its start, nm.
    pub along_nm: f64,
    pub alt_ft: f64,
    /// At the field.
    pub landed: bool,
}

/// A named fix on the route ahead, with its distance along [`Trajectory::path`] (nm).
pub struct RouteFix {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub along_nm: f64,
}

/// One flight, resolved once, that every surface reads its times from.
///
/// The route is resolved when [`trajectory`] returns; the vertical profile and the winds are computed
/// the first time a time or speed is asked for, so a caller that only tests the route (an FCA
/// crossing, a sector box) pays nothing for flights it drops.
pub struct Trajectory<'a> {
    inp: &'a Inputs<'a>,
    fp: &'a FlightPlan,
    origin: Option<Origin>,
    path: Vec<[f64; 2]>,
    airborne: bool,
    scale: f64,
    route_nm: f64,
    released: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    timing: OnceCell<Timing>,
}

struct Timing {
    vp: trajectory::VerticalProfile,
    cruise_ft: f64,
    cruise_tas: f64,
    wheels_up: DateTime<Utc>,
}

/// Resolve `flight` for timing at `now`. `released` is the wheels-up a ground flight holds (a ready
/// time, an FCA release, a CFR or a GDP EDCT); it replaces the proposed wheels-up. With `None` the
/// flight's lock in `inp.locked` is used, so no caller can time a locked flight as proposed by
/// leaving it out. `None` when the flight has no flight plan, is airborne with neither a route nor a
/// known destination, or is a prefile that holds no lock: a prefile gets no proposed wheels-up (#814).
pub fn trajectory<'a>(
    inp: &'a Inputs<'a>,
    flight: Flight<'a>,
    released: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Option<Trajectory<'a>> {
    let callsign = match flight {
        Flight::Pilot(p) => p.callsign.as_str(),
        Flight::Prefile(pf) => pf.callsign.as_str(),
    };
    let released = released.or_else(|| inp.locked_wheels_up(callsign));
    let (fp, prefile, origin) = match flight {
        Flight::Pilot(p) => (
            p.flight_plan.as_ref()?,
            false,
            Some(Origin {
                pos: [p.latitude, p.longitude],
                hdg: p.heading,
                gs: p.groundspeed,
                alt_ft: p.altitude as f64,
            }),
        ),
        Flight::Prefile(pf) => {
            released?;
            let fp = pf.flight_plan.as_ref()?;
            let dep = inp.airports.get(&fp.departure.to_ascii_uppercase());
            let origin = dep.map(|a| Origin {
                pos: [a.lat, a.lon],
                hdg: 0,
                gs: 0,
                alt_ft: 0.0,
            });
            (fp, true, origin)
        }
    };
    let airborne = origin.is_some_and(|o| is_airborne(o.gs));
    let arr_ll = inp
        .airports
        .get(&fp.arrival.to_ascii_uppercase())
        .map(|a| [a.lat, a.lon]);
    let resolved = origin.and_then(|o| {
        resolve_path(
            inp.nav,
            inp.airports,
            (&fp.departure, &fp.arrival, &fp.route),
            (o.pos, o.hdg, o.gs),
            arr_ll,
            !prefile,
        )
    });
    let (path, scale, route_nm) = match resolved {
        Some((path, enroute)) => {
            let scale = route_scale(airborne, enroute);
            let route_nm = path_len_nm(&path) * scale;
            (path, scale, route_nm)
        }
        None if airborne => return None,
        None => (Vec::new(), 1.0, NOMINAL_ROUTE_NM),
    };
    Some(Trajectory {
        inp,
        fp,
        origin,
        path,
        airborne,
        scale,
        route_nm,
        released,
        now,
        timing: OnceCell::new(),
    })
}

/// The path [`trajectory`] times a flight at `pos` along, for a caller that only needs the crossing
/// geometry (the FCA badge counts, the flight advisory's pre-filter): the filed route, else the
/// great circle to the destination (none for a pilot that has arrived, as [`resolve_path`]).
pub fn flight_path(
    nav: &NavData,
    airports: &AirportDb,
    fp: &FlightPlan,
    pos: [f64; 2],
    hdg: i64,
    gs: i64,
) -> Option<Vec<[f64; 2]>> {
    let arr_ll = airports
        .get(&fp.arrival.to_ascii_uppercase())
        .map(|a| [a.lat, a.lon]);
    let route = (
        fp.departure.as_str(),
        fp.arrival.as_str(),
        fp.route.as_str(),
    );
    resolve_path(nav, airports, route, (pos, hdg, gs), arr_ll, true).map(|(path, _)| path)
}

/// A connected, not-yet-airborne pilot this close (nm) to an arrival field other than its departure
/// field has arrived there: no surface times it. Airport flow labels a ground pilot this close
/// "arrived" whatever its departure field.
pub(crate) const ARRIVED_NM: f64 = 5.0;

/// The path a flight at `pos` is timed along, and whether the nav engine placed an enroute point on
/// it (`> 2` anchors; exactly 2 is no better than the great circle). Falls back to the great circle
/// to `arr_ll` when the filed route does not resolve, except for a connected `pilot` on the ground
/// within [`ARRIVED_NM`] of an arrival field it did not depart from: `route_path` returns `None` for
/// it because it has arrived, and it has nowhere left to fly. A local flight (departure = arrival)
/// starts at its arrival field without having arrived, and a prefile has not flown at all.
/// [`arrival_route`] passes `pilot = false`: it falls back to the great circle itself.
fn resolve_path(
    nav: &NavData,
    airports: &AirportDb,
    (dep, arr, route): (&str, &str, &str),
    (pos, hdg, gs): ([f64; 2], i64, i64),
    arr_ll: Option<[f64; 2]>,
    pilot: bool,
) -> Option<(Vec<[f64; 2]>, bool)> {
    let can_arrive = pilot && !is_airborne(gs) && !dep.eq_ignore_ascii_case(arr);
    match fca::route_path(nav, airports, dep, arr, route, pos[0], pos[1], hdg, gs) {
        Some(path) => {
            let enroute = path.len() > 2;
            Some((path, enroute))
        }
        None => arr_ll
            .filter(|a| !can_arrive || gc_dist(pos[0], pos[1], a[0], a[1]) >= ARRIVED_NM)
            .map(|a| (vec![pos, a], false)),
    }
}

/// The along-route length multiplier: [`GROUND_ROUTE_FACTOR`] for a not-yet-airborne flight whose
/// route did not resolve past its endpoints, else 1. An airborne flight is timed on the route the
/// map draws, so padding it would double-correct.
fn route_scale(airborne: bool, enroute_resolved: bool) -> f64 {
    if !airborne && !enroute_resolved {
        GROUND_ROUTE_FACTOR
    } else {
        1.0
    }
}

/// The filed cruise speed as a number, `0` when blank or junk; [`trajectory::capped_cruise_tas`]
/// then replaces an implausible value with the profile's or the altitude default.
fn filed_tas(raw: &str) -> f64 {
    raw.trim().parse().unwrap_or(0.0)
}

/// A filed departure time (`deptime`, HHMM Z) as its nearest occurrence within 12 hours of `now`, so
/// `2350` just after midnight is last night (vTBFM manual §8.6). `None` when it is blank, malformed
/// or `0000`, which a pilot client files when nobody chose a time.
pub(crate) fn filed_departure(deptime: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let d = deptime.trim();
    if d.len() != 4 || !d.bytes().all(|c| c.is_ascii_digit()) || d == "0000" {
        return None;
    }
    let (h, m) = (d[..2].parse().ok()?, d[2..].parse().ok()?);
    let t = now.date_naive().and_hms_opt(h, m, 0)?.and_utc();
    let half_day = Duration::hours(12);
    Some(if t - now > half_day {
        t - Duration::days(1)
    } else if now - t > half_day {
        t + Duration::days(1)
    } else {
        t
    })
}

/// The wheels-up of a connected ground pilot that holds no scheduler time: its filed departure plus
/// [`PROPOSED_WHEELS_UP_MIN`]. A filed time already past, or one that does not read, counts as now
/// (#814).
pub(crate) fn proposed_wheels_up(deptime: &str, now: DateTime<Utc>) -> DateTime<Utc> {
    let pushback = filed_departure(deptime, now).map_or(now, |t| t.max(now));
    pushback + Duration::minutes(PROPOSED_WHEELS_UP_MIN)
}

impl Trajectory<'_> {
    /// The route the flight is timed along, from where it is (or its departure) to the field. Empty
    /// for a ground flight timed over the nominal leg.
    pub fn path(&self) -> &[[f64; 2]] {
        &self.path
    }

    pub fn airborne(&self) -> bool {
        self.airborne
    }

    /// The along-route distance still to fly, nm, padding included.
    pub fn route_nm(&self) -> f64 {
        self.route_nm
    }

    /// A distance along [`Self::path`] as the distance the model flies, padding included.
    pub fn modeled_nm(&self, along_nm: f64) -> f64 {
        along_nm * self.scale
    }

    /// ETA at the point `along_nm` along [`Self::path`].
    pub fn eta_at(&self, along_nm: f64) -> DateTime<Utc> {
        self.eta_to_go(self.to_go(along_nm))
    }

    /// Predicted groundspeed (kt) at the point `along_nm` along [`Self::path`].
    pub fn gs_at(&self, along_nm: f64) -> f64 {
        self.timing().vp.ground_speed_at(self.to_go(along_nm))
    }

    /// Predicted altitude (ft) at the point `along_nm` along [`Self::path`].
    pub fn alt_at(&self, along_nm: f64) -> f64 {
        self.timing().vp.alt_at(self.to_go(along_nm))
    }

    /// ETA at the destination field.
    pub fn eta_at_field(&self) -> DateTime<Utc> {
        self.eta_to_go(0.0)
    }

    /// Predicted groundspeed (kt) `nm` along-route short of the field. A flight already closer than
    /// that has no such point ahead of it, so it reads its own descent schedule `nm` out, in the wind
    /// along its remaining route, held at its position beyond it (#813).
    pub fn gs_short_of_field(&self, nm: f64) -> f64 {
        let t = self.timing();
        if nm <= self.route_nm {
            return t.vp.ground_speed_at(nm);
        }
        let fp = self.fp;
        let (ty, wake) = fp.aircraft_type_wake();
        trajectory::descent_ground_speed_at(
            nm,
            field_elevation_ft(self.inp.airports, &fp.arrival),
            t.cruise_ft,
            t.cruise_tas,
            self.inp.profiles.resolve(&ty, &wake),
            &self.inp.winds.along(&self.path, self.route_nm),
        )
    }

    /// Where the flight is after `flying_sec` from its start: now for an airborne flight, wheels-up
    /// for one on the ground.
    pub fn progress_after(&self, flying_sec: f64) -> Progress {
        let vp = &self.timing().vp;
        let d = vp.distance_after(self.route_nm, flying_sec.max(0.0));
        Progress {
            along_nm: ((self.route_nm - d) / self.scale).max(0.0),
            alt_ft: vp.alt_at(d),
            landed: d <= 0.0,
        }
    }

    /// When the flight leaves the ground: now for an airborne one.
    pub fn wheels_up(&self) -> DateTime<Utc> {
        self.timing().wheels_up
    }

    pub fn cruise_ft(&self) -> f64 {
        self.timing().cruise_ft
    }

    pub fn cruise_tas(&self) -> f64 {
        self.timing().cruise_tas
    }

    /// Distance-weighted mean headwind (kt) over the route still to fly, for the FCA debug view.
    pub fn mean_headwind(&self) -> Option<f64> {
        self.timing().vp.mean_headwind(self.route_nm, 0.0)
    }

    /// The named fixes ahead on the route, resolved from the same position [`Self::path`] was.
    pub fn fixes(&self) -> Vec<RouteFix> {
        let Some(o) = self.origin else {
            return Vec::new();
        };
        let fp = self.fp;
        fca::route_path_named(
            self.inp.nav,
            self.inp.airports,
            &fp.departure,
            &fp.arrival,
            &fp.route,
            o.pos[0],
            o.pos[1],
            o.hdg,
            o.gs,
        )
        .unwrap_or_default()
        .into_iter()
        .map(|(name, lat, lon, along_nm)| RouteFix {
            name,
            lat,
            lon,
            along_nm,
        })
        .collect()
    }

    /// The model's distance to the field (nm) at `along_nm` along the path.
    fn to_go(&self, along_nm: f64) -> f64 {
        (self.route_nm - along_nm * self.scale).max(0.0)
    }

    fn eta_to_go(&self, to_go_nm: f64) -> DateTime<Utc> {
        let t = self.timing();
        let sec = t.vp.time_between(self.route_nm, to_go_nm);
        t.wheels_up + Duration::seconds(sec as i64)
    }

    fn timing(&self) -> &Timing {
        self.timing.get_or_init(|| self.time())
    }

    fn time(&self) -> Timing {
        let (inp, fp) = (self.inp, self.fp);
        let (ty, wake) = fp.aircraft_type_wake();
        let profile = inp.profiles.resolve(&ty, &wake);
        let cruise_ft = trajectory::parse_alt_ft(&fp.altitude);
        let cruise_tas =
            trajectory::capped_cruise_tas(filed_tas(&fp.cruise_tas), cruise_ft, profile);
        let wind = inp.winds.along(&self.path, self.route_nm);
        let (alt_ft, gs) = self.origin.map_or((0.0, 0.0), |o| (o.alt_ft, o.gs as f64));
        let vp = profile_from_here(
            self.airborne,
            self.route_nm,
            alt_ft,
            gs,
            cruise_ft,
            cruise_tas,
            inp.airports,
            &fp.arrival,
            profile,
            wind,
        );
        let wheels_up = match self.released {
            _ if self.airborne => self.now,
            Some(r) => r.max(self.now),
            // Only a pilot reaches here: `trajectory` refuses a prefile with no lock.
            None => proposed_wheels_up(&fp.deptime, self.now),
        };
        Timing {
            vp,
            cruise_ft,
            cruise_tas,
            wheels_up,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::airports::Airport;
    use std::collections::HashMap;
    use std::sync::LazyLock;

    fn airports() -> AirportDb {
        HashMap::from([
            ("KJFK".to_string(), Airport::at(40.64, -73.78)),
            ("KMIA".to_string(), Airport::at(25.79, -80.29)),
        ])
    }

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).unwrap()
    }

    /// KJFK→KMIA, MIA at (25.79, -80.29). `pos`/`alt`/`gs` describe the aircraft.
    fn input(pos: [f64; 2], alt_ft: f64, gs: i64) -> ArrivalInput<'static> {
        ArrivalInput {
            dep: "KJFK",
            arr: "KMIA",
            route: "",
            pos,
            alt_ft,
            gs,
            hdg: 190,
            arr_ll: [25.79, -80.29],
            cruise_ft: 35_000.0,
            cruise_tas: 440.0,
        }
    }

    fn predict(ac: &ArrivalInput) -> ArrivalPrediction {
        arrival_eta(
            &NavData::default(),
            &airports(),
            &Winds::default(),
            &AircraftProfile::default(),
            ac,
            now(),
        )
    }

    /// AC #3: one aircraft yields the same ETA via the arrival (ladder) path and the crossing
    /// (FCA) path — timed off the **resolved filed route**, not a straight line. Uses the bundled
    /// nav db and a real NJ-coast routing (`RBV WHITE SIE`): `arrival_eta` (the ladder/runway
    /// path) must agree to the instant with `eta_along_route` timed at the field (the FCA path),
    /// its distance must equal the independently resolved route length, and that length must be
    /// meaningfully longer than the great circle to the field. Reverting `arrival_eta` to
    /// `gc_dist` breaks the distance assertions.
    #[test]
    fn arrival_and_crossing_eta_agree_on_the_resolved_route() {
        let nav = NavData::load();
        let profile = AircraftProfile::default();
        let ap: AirportDb = HashMap::from([
            ("KJFK".to_string(), Airport::at(40.64, -73.78)),
            ("KDCA".to_string(), Airport::at(38.85, -77.04)),
        ]);
        // Airborne B738 just south of KJFK tracking SW down the coast, filed KJFK -> KDCA.
        let pos = [40.2, -74.0];
        let ac = ArrivalInput {
            dep: "KJFK",
            arr: "KDCA",
            route: "RBV WHITE SIE",
            hdg: 220,
            arr_ll: [38.85, -77.04],
            ..input(pos, 24_000.0, 400)
        };
        let pred = arrival_eta(&nav, &ap, &Winds::default(), &profile, &ac, now());

        // Distance follows the resolved route, exactly — and it's longer than the straight line.
        let path = fca::route_path(
            &nav, &ap, ac.dep, ac.arr, ac.route, pos[0], pos[1], ac.hdg, ac.gs,
        )
        .expect("route resolves");
        let route_len = path_len_nm(&path);
        let straight = gc_dist(pos[0], pos[1], ac.arr_ll[0], ac.arr_ll[1]);
        assert!(
            (pred.route_nm - route_len).abs() < 1e-6,
            "arrival_eta must follow route_path"
        );
        assert!(
            pred.route_nm > straight + 20.0,
            "resolved route {:.0} nm should exceed the straight line {straight:.0} nm",
            pred.route_nm
        );

        // FCA path: same resolved route, crossing timed at the field (along == route_len).
        let hw = Winds::default().along(&path, route_len);
        let fca_eta = eta_along_route(
            true,
            route_len,
            route_len,
            ac.alt_ft,
            ac.gs as f64,
            ac.cruise_ft,
            ac.cruise_tas,
            &airports(),
            "KMIA",
            &profile,
            hw,
            now(),
        );
        assert_eq!(
            pred.eta, fca_eta.eta,
            "ladder ETA must equal the FCA crossing-at-field ETA"
        );
    }

    /// A not-yet-airborne flight with a bare/unresolvable route keeps the great-circle → route
    /// padding, so its metered wheels-up / CFR time isn't computed on an unachievable distance.
    /// An airborne aircraft with the same unresolved route is timed on the raw straight line
    /// (that's the route the map draws).
    #[test]
    fn ground_flight_with_unresolved_route_keeps_the_route_padding() {
        let straight = gc_dist(40.64, -73.78, 25.79, -80.29);

        let ground = predict(&input([40.64, -73.78], 0.0, 0)); // route: "" -> only endpoints
        assert!(
            (ground.route_nm - straight * GROUND_ROUTE_FACTOR).abs() < 1e-6,
            "ground route_nm {:.1} should be the padded {:.1}",
            ground.route_nm,
            straight * GROUND_ROUTE_FACTOR
        );

        // Airborne, same unresolved route: no padding — timed on the straight line to the field.
        let airborne = predict(&input([31.0, -80.5], 35_000.0, 440));
        let straight_air = gc_dist(31.0, -80.5, 25.79, -80.29);
        assert!(
            (airborne.route_nm - straight_air).abs() < 1.0,
            "airborne route_nm {:.1} should be ~the straight line {:.1}, unpadded",
            airborne.route_nm,
            straight_air
        );
    }

    #[test]
    fn ground_flight_at_a_realistic_gate_offset_still_keeps_the_padding() {
        // Regression guard (#213): a real gate/ramp position is never exactly the airport's
        // reference point. fca.rs's ground-route trimming must recognize "still at the departure
        // airport" structurally (by index), not by a near-exact coordinate match — otherwise
        // padding silently drops for ordinary pre-departure traffic once it's off the ARP by even
        // a few tenths of a mile.
        let gate = [40.635, -73.775]; // ~0.4 nm from the KJFK reference point `input` resolves to
        let straight = gc_dist(gate[0], gate[1], 25.79, -80.29);
        let ground = predict(&input(gate, 0.0, 0));
        assert!(
            (ground.route_nm - straight * GROUND_ROUTE_FACTOR).abs() < 1e-6,
            "ground route_nm {:.1} should be the padded {:.1} even from a gate offset from the ARP",
            ground.route_nm,
            straight * GROUND_ROUTE_FACTOR
        );
    }

    #[test]
    fn unresolvable_route_falls_back_to_straight_line() {
        // Unknown airports → route_path yields < 2 anchors → straight-line fallback.
        let ac = ArrivalInput {
            dep: "ZZZZ",
            arr: "YYYY",
            ..input([30.0, -80.0], 35_000.0, 440)
        };
        let pred = predict(&ac);
        let expect = gc_dist(30.0, -80.0, 25.79, -80.29);
        assert!((pred.route_nm - expect).abs() < 1e-6);
        assert!(pred.eta > now());
    }

    #[test]
    fn a_ground_flight_flies_the_whole_route_from_the_surface() {
        let airborne = predict(&input([30.5, -80.0], 35_000.0, 440));
        let ground = predict(&input([40.64, -73.78], 0.0, 0));
        // The ground flight flies the whole route from the surface, so it's later.
        assert!(ground.eta > airborne.eta);
        assert!(ground.route_nm > airborne.route_nm);
    }

    /// #411: `profile_from_here` is the one place an airborne prediction is anchored to its
    /// observed groundspeed, and nothing proved that wiring — deleting the `if airborne` branch
    /// left all 527 tests green.
    #[test]
    fn an_airborne_prediction_is_anchored_to_the_observed_groundspeed() {
        let profile = AircraftProfile::default();
        let eta_at = |airborne: bool, observed_gs: f64| {
            eta_along_route(
                airborne,
                300.0,
                300.0,
                if airborne { 35_000.0 } else { 0.0 },
                observed_gs,
                35_000.0,
                440.0,
                &airports(),
                "KMIA",
                &profile,
                RouteWind::still(),
                now(),
            )
            .eta
        };

        // Established at cruise (within ANCHOR_ALT_TOLERANCE_FT of it, above ANCHOR_MIN_GS_KT) and
        // 30 kt faster than the 440 kt profile, so every predicted groundspeed scales to it.
        let (fast, on_profile) = (eta_at(true, 470.0), eta_at(true, 440.0));
        assert!(
            fast < on_profile,
            "an aircraft observed at 470 kt must arrive before one flying the 440 kt profile, got \
             {fast} vs {on_profile} — equal means the anchoring never reached the profile"
        );

        // A ground aircraft is never anchored, so its observed groundspeed is inert.
        assert_eq!(
            eta_at(false, 470.0),
            eta_at(false, 440.0),
            "a ground aircraft must not be anchored to its observed groundspeed"
        );
    }

    /// #315: the arrival ETA descends to the destination's real field elevation, and an unknown
    /// destination is timed exactly as a sea-level field.
    #[test]
    fn arrival_eta_descends_to_the_destination_field_elevation() {
        let at = |elevation_ft| {
            let ap: AirportDb = HashMap::from([
                ("KJFK".to_string(), Airport::at(40.64, -73.78)),
                (
                    "KMIA".to_string(),
                    Airport {
                        elevation_ft,
                        ..Airport::at(25.79, -80.29)
                    },
                ),
            ]);
            arrival_eta(
                &NavData::default(),
                &ap,
                &Winds::default(),
                &AircraftProfile::default(),
                &input([35.0, -77.0], 35_000.0, 450),
                now(),
            )
            .eta
        };
        let sea_level = at(0.0);
        // #335: the *direction*, not merely "different". A higher field shortens the descent, so
        // the arrival is strictly earlier — `assert_ne!` was also satisfied by an inverted sign.
        assert!(
            at(5431.0) < sea_level,
            "a 5431 ft field must arrive earlier than a sea-level one, got {} vs {sea_level}",
            at(5431.0)
        );

        let unknown_dest = ArrivalInput {
            arr: "ZZZZ",
            ..input([35.0, -77.0], 35_000.0, 450)
        };
        let unknown = arrival_eta(
            &NavData::default(),
            &airports(),
            &Winds::default(),
            &AircraftProfile::default(),
            &unknown_dest,
            now(),
        )
        .eta;
        assert_eq!(unknown, sea_level);
    }

    /// #355: `eta_along_route` must report the groundspeed **at the target fix**, not cruise.
    /// FCA metering converts a MIT distance into a time gap with this number and then freezes it at
    /// release, so a cruise-speed figure under-provisions every arrival that crosses on descent.
    /// A low metering fix (20 nm from the field, on the descent) must come back far below cruise.
    #[test]
    fn eta_along_route_reports_the_descent_speed_at_a_low_crossing_fix() {
        let profile = AircraftProfile::default();
        let cruise_tas = 440.0;
        let route_len = 300.0;
        let cruise_gs = trajectory::effective_gs(cruise_tas, None);

        // Target 280 nm ahead of an aircraft 300 nm out = a fix 20 nm from the field, well down
        // the descent.
        let at_fix = eta_along_route(
            true,
            route_len,
            280.0,
            35_000.0,
            0.0,
            35_000.0,
            cruise_tas,
            &airports(),
            "KMIA",
            &profile,
            RouteWind::still(),
            now(),
        );

        assert!(
            at_fix.gs_kt < cruise_gs * 0.8,
            "crossing speed {:.0} kt must be well below cruise {cruise_gs:.0} kt, not cruise itself",
            at_fix.gs_kt
        );
        assert!(
            (150.0..=350.0).contains(&at_fix.gs_kt),
            "crossing speed {:.0} kt should sit in a plausible arrival band",
            at_fix.gs_kt
        );

        // The bug in one assertion: spacing 20 MIT with the cruise figure realizes ~13 nm at the
        // fix, because the aircraft covers the frozen gap at its actual crossing speed.
        let realized_with_cruise = 20.0 * (at_fix.gs_kt / cruise_gs);
        assert!(
            realized_with_cruise < 15.0,
            "cruise-sized 20 MIT would realize only {realized_with_cruise:.1} nm"
        );
    }

    /// The companion to the above: the speed is genuinely position-dependent, so a fix still at
    /// cruise reports a cruise-like speed. Guards against "always return a slow number" passing
    /// the descent assertion for the wrong reason.
    #[test]
    fn eta_along_route_still_reports_cruise_speed_at_an_enroute_fix() {
        let profile = AircraftProfile::default();
        let at_cruise = eta_along_route(
            true,
            300.0,
            150.0,
            35_000.0,
            0.0,
            35_000.0,
            440.0,
            &airports(),
            "KMIA",
            &profile,
            RouteWind::still(),
            now(),
        );
        let at_fix = eta_along_route(
            true,
            300.0,
            280.0,
            35_000.0,
            0.0,
            35_000.0,
            440.0,
            &airports(),
            "KMIA",
            &profile,
            RouteWind::still(),
            now(),
        );
        assert!(
            at_cruise.gs_kt > at_fix.gs_kt + 100.0,
            "enroute {:.0} kt should far exceed the descent fix {:.0} kt",
            at_cruise.gs_kt,
            at_fix.gs_kt
        );
    }

    // ---- #836: `arrival_eta` reads the wind per segment (AC3, AC5) ----------------------------
    //
    // Every caller test seeds `Winds::test_shear()`: 80 kt from 225° (on the nose of a
    // south-west KJFK→KDCA course) at and above 30,000 ft east of 75.5°W, calm elsewhere.

    fn jfk_dca() -> AirportDb {
        HashMap::from([
            ("KJFK".to_string(), Airport::at(40.64, -73.78)),
            ("KDCA".to_string(), Airport::at(38.85, -77.04)),
        ])
    }

    /// The airborne B738 of `arrival_and_crossing_eta_agree_on_the_resolved_route`: just south
    /// of KJFK tracking SW down the coast, FL240 climbing to 35,000.
    fn coastal(route: &'static str) -> ArrivalInput<'static> {
        ArrivalInput {
            dep: "KJFK",
            arr: "KDCA",
            route,
            hdg: 220,
            arr_ll: [38.85, -77.04],
            ..input([40.2, -74.0], 24_000.0, 400)
        }
    }

    fn eta_in(nav: &NavData, winds: &Winds, ac: &ArrivalInput) -> DateTime<Utc> {
        let profile = AircraftProfile::default();
        arrival_eta(nav, &jfk_dca(), winds, &profile, ac, now()).eta
    }

    fn secs(a: DateTime<Utc>, b: DateTime<Utc>) -> i64 {
        (a - b).num_seconds()
    }

    // AC5: the resolved-route site uses the wind along the resolved path.
    #[test]
    fn arrival_eta_times_a_resolved_route_through_the_wind_along_it() {
        let nav = NavData::load();
        let ac = coastal("RBV WHITE SIE");
        let shear = Winds::test_shear();
        let windy = eta_in(&nav, &shear, &ac);
        let still = eta_in(&nav, &Winds::default(), &ac);
        assert!(
            secs(windy, still) > 30,
            "the headwind delays it {} s",
            secs(windy, still)
        );
        let path = fca::route_path(
            &nav,
            &jfk_dca(),
            "KJFK",
            "KDCA",
            ac.route,
            40.2,
            -74.0,
            220,
            400,
        )
        .expect("route resolves");
        let len = path_len_nm(&path);
        let crossing = eta_along_route(
            true,
            len,
            len,
            ac.alt_ft,
            400.0,
            ac.cruise_ft,
            ac.cruise_tas,
            &jfk_dca(),
            "KDCA",
            &AircraftProfile::default(),
            shear.along(&path, len),
            now(),
        );
        assert_eq!(windy, crossing.eta, "ladder and FCA agree under wind");
    }

    // AC5: the straight-line fallback site (no nav) uses the wind on the great circle.
    #[test]
    fn arrival_eta_times_an_unresolved_route_through_the_wind_on_the_great_circle() {
        // KDCA is missing from the airport database, so the route cannot resolve at all and the
        // caller's `arr_ll` is the only destination.
        let nav = NavData::default();
        let ap: AirportDb = HashMap::from([("KJFK".to_string(), Airport::at(40.64, -73.78))]);
        let ac = coastal("");
        assert!(fca::route_path(&nav, &ap, "KJFK", "KDCA", "", 40.2, -74.0, 220, 400).is_none());
        let shear = Winds::test_shear();
        let eta =
            |w: &Winds| arrival_eta(&nav, &ap, w, &AircraftProfile::default(), &ac, now()).eta;
        let (windy, still) = (eta(&shear), eta(&Winds::default()));
        assert!(secs(windy, still) > 30, "{} s", secs(windy, still));
        let len = gc_dist(40.2, -74.0, 38.85, -77.04);
        let direct = eta_along_route(
            true,
            len,
            len,
            ac.alt_ft,
            400.0,
            ac.cruise_ft,
            ac.cruise_tas,
            &ap,
            "KDCA",
            &AircraftProfile::default(),
            shear.along(&[[40.2, -74.0], [38.85, -77.04]], len),
            now(),
        );
        assert_eq!(windy, direct.eta);
    }

    // The ground padding (GROUND_ROUTE_FACTOR) is applied before the wind is laid on the profile,
    // so the departure-end wind lands on the climb and the arrival-end calm on the descent.
    #[test]
    fn a_ground_flights_wind_spans_the_padded_route_from_the_departure() {
        let nav = NavData::default();
        let ac = ArrivalInput {
            pos: [40.64, -73.78],
            gs: 0,
            alt_ft: 0.0,
            ..coastal("")
        };
        let shear = Winds::test_shear();
        let eta = eta_in(&nav, &shear, &ac);
        let gc = gc_dist(40.64, -73.78, 38.85, -77.04);
        let padded = gc * GROUND_ROUTE_FACTOR;
        let line = [[40.64, -73.78], [38.85, -77.04]];
        let at = |wind_len: f64| {
            eta_along_route(
                false,
                padded,
                padded,
                0.0,
                0.0,
                ac.cruise_ft,
                ac.cruise_tas,
                &jfk_dca(),
                "KDCA",
                &AircraftProfile::default(),
                shear.along(&line, wind_len),
                now(),
            )
            .eta
        };
        assert_eq!(eta, at(padded));
        assert_ne!(at(padded), at(gc), "the padding moves where the wind sits");
    }

    // AC3 at the site: the route-mean value the old code used gives a different answer.
    #[test]
    fn arrival_eta_differs_from_a_route_mean_wind() {
        let nav = NavData::load();
        let ac = coastal("RBV WHITE SIE");
        let winds = Winds::test_shear();
        let shear = eta_in(&nav, &winds, &ac);
        let still = eta_in(&nav, &Winds::default(), &ac);
        assert!(
            secs(shear, still) > 30,
            "positive control: {} s",
            secs(shear, still)
        );
        // The pre-#836 model: one mean headwind at cruise over the segment midpoints, applied to
        // climb, cruise and descent alike.
        let path = fca::route_path(
            &nav,
            &jfk_dca(),
            "KJFK",
            "KDCA",
            ac.route,
            40.2,
            -74.0,
            220,
            400,
        )
        .unwrap();
        let mids: Vec<f64> = path
            .windows(2)
            .filter_map(|s| {
                let course = fca::bearing_deg(s[0], s[1]);
                let (la, lo) = ((s[0][0] + s[1][0]) / 2.0, (s[0][1] + s[1][1]) / 2.0);
                winds.point_headwind(la, lo, course, ac.cruise_ft)
            })
            .collect();
        let mean_hw = mids.iter().sum::<f64>() / mids.len() as f64;
        let len = path_len_nm(&path);
        let old = eta_along_route(
            true,
            len,
            len,
            ac.alt_ft,
            400.0,
            ac.cruise_ft,
            ac.cruise_tas,
            &jfk_dca(),
            "KDCA",
            &AircraftProfile::default(),
            RouteWind::uniform(mean_hw),
            now(),
        )
        .eta;
        // The descent flies through calmer air than the cruise mean, so the two differ.
        assert!(
            secs(shear, old).abs() > 5,
            "per segment vs the {mean_hw:.0} kt route mean: {} s",
            secs(shear, old)
        );
    }

    // The FCA debug view reads its mean headwind from the profile that timed the crossing.
    #[test]
    fn eta_along_route_returns_the_profile_its_eta_was_timed_on() {
        let pred = eta_along_route(
            true,
            300.0,
            100.0,
            35_000.0,
            0.0,
            35_000.0,
            440.0,
            &airports(),
            "KMIA",
            &AircraftProfile::default(),
            RouteWind::uniform(60.0),
            now(),
        );
        let timed = pred.profile.time_between(300.0, 200.0);
        assert!(((pred.eta - now()).num_seconds() as f64 - timed).abs() <= 1.0);
        assert_eq!(pred.profile.ground_speed_at(200.0), pred.gs_kt);
        assert_eq!(pred.profile.mean_headwind(300.0, 0.0), Some(60.0));
    }

    // ---- #813 AC2: the entry point decides each input once -------------------------------------

    fn plan(dep: &str, arr: &str, route: &str) -> FlightPlan {
        FlightPlan {
            departure: dep.into(),
            arrival: arr.into(),
            route: route.into(),
            aircraft_short: "B738".into(),
            altitude: "FL350".into(),
            cruise_tas: "440".into(),
            ..Default::default()
        }
    }

    fn pilot(pos: [f64; 2], alt: i64, gs: i64, fp: FlightPlan) -> Pilot {
        Pilot {
            callsign: "AAL1".into(),
            latitude: pos[0],
            longitude: pos[1],
            altitude: alt,
            groundspeed: gs,
            heading: 190,
            flight_plan: Some(fp),
            ..Default::default()
        }
    }

    fn prefile(fp: FlightPlan) -> Prefile {
        Prefile {
            callsign: "PRE1".into(),
            flight_plan: Some(fp),
            ..Default::default()
        }
    }

    /// Inputs over `nav` and `ap`, still air and the default profiles.
    fn still_air<'a>(nav: &'a NavData, ap: &'a AirportDb) -> Inputs<'a> {
        static WINDS: LazyLock<Winds> = LazyLock::new(Winds::default);
        static PROFILES: LazyLock<ProfileTable> = LazyLock::new(ProfileTable::default);
        Inputs {
            nav,
            airports: ap,
            winds: &WINDS,
            profiles: &PROFILES,
            locked: &NO_LOCKS,
        }
    }

    const JFK: [f64; 2] = [40.64, -73.78];

    /// A route that does not resolve falls back to the great circle to the destination, except for a
    /// pilot on the ground at an arrival field other than its departure field, which has arrived: `route_path`'s `None` there means
    /// nothing is left to fly, and a great circle would time it as a fresh departure.
    #[test]
    fn a_landed_pilot_gets_no_great_circle_to_its_own_field() {
        let (nav, ap) = (NavData::default(), airports());
        let path = |pos: [f64; 2], gs: i64| {
            let fp = plan("KZZZ", "KMIA", "DCT");
            flight_path(&nav, &ap, &fp, pos, 190, gs)
        };
        assert_eq!(path([25.80, -80.29], 0), None, "parked at KMIA");
        // 0.0833 deg of longitude at 25.79 N is 4.5 nm; 0.10 deg is 5.4 nm.
        assert_eq!(
            path([25.79, -80.2067], 0),
            None,
            "4.5 nm out, on the ground"
        );
        assert_eq!(
            path([25.79, -80.19], 0),
            Some(vec![[25.79, -80.19], [25.79, -80.29]]),
            "5.4 nm out, still on the ground"
        );
        assert_eq!(
            path(JFK, 0).map(|p| p.len()),
            Some(2),
            "a departure far away"
        );
        assert_eq!(
            path([25.84, -80.29], 140).map(|p| p.len()),
            Some(2),
            "an airborne arrival 3 nm out"
        );
        assert_eq!(ARRIVED_NM, 5.0);
        let inp = still_air(&nav, &ap);
        let landed = pilot([25.80, -80.29], 0, 0, plan("KZZZ", "KMIA", "DCT"));
        let traj = trajectory(&inp, Flight::Pilot(&landed), None, now()).unwrap();
        assert!(traj.path().is_empty(), "nothing to cross");
    }

    /// A local flight (departure = arrival) and a prefile both start at a field without having
    /// arrived there, so they keep a path, and not the 300 nm nominal leg a
    /// flight with no route at all gets.
    #[test]
    fn a_local_flight_and_a_prefile_have_not_arrived() {
        let (nav, ap) = (NavData::default(), airports());
        let inp = still_air(&nav, &ap);
        let local = plan("kmia", "KMIA", "DCT");
        let pre = prefile(local.clone());
        let t = trajectory(&inp, Flight::Prefile(&pre), Some(now()), now()).unwrap();
        assert_eq!(
            t.path(),
            [[25.79, -80.29], [25.79, -80.29]],
            "local prefile"
        );
        assert_eq!(
            t.route_nm(),
            0.0,
            "not the {NOMINAL_ROUTE_NM} nm nominal leg"
        );
        let parked = pilot([25.80, -80.29], 0, 0, local.clone());
        let t = trajectory(&inp, Flight::Pilot(&parked), None, now()).unwrap();
        assert_eq!(t.path().len(), 2, "local pilot parked at its field");
        assert_eq!(
            flight_path(&nav, &ap, &local, [25.80, -80.29], 190, 0).map(|p| p.len()),
            Some(2),
            "the badge counts see the same local pilot"
        );
        // A field 2.0 nm from KMIA, inside its arrived radius. `route_path` collapses a ground leg
        // under 3 nm, so only the great circle times a flight from it.
        let mut near_ap = airports();
        near_ap.insert("KNBR".into(), Airport::at(25.79, -80.253));
        let inp = still_air(&nav, &near_ap);
        let near = prefile(plan("KNBR", "KMIA", "DCT"));
        let t = trajectory(&inp, Flight::Prefile(&near), Some(now()), now()).unwrap();
        assert_eq!(
            t.path(),
            [[25.79, -80.253], [25.79, -80.29]],
            "a prefile has not arrived"
        );
        let parked = pilot([25.79, -80.253], 0, 0, plan("KNBR", "KMIA", "DCT"));
        let t = trajectory(&inp, Flight::Pilot(&parked), None, now()).unwrap();
        assert!(
            t.path().is_empty(),
            "control: a pilot on the ground there has"
        );
    }

    /// AC2 airborne: from 50 kt, whatever the altitude, and the altitude alone never makes a flight
    /// airborne. 50-60 kt was ground to airport flow and projected traffic, airborne to FCA.
    #[test]
    fn a_flight_is_airborne_from_50_kt_at_any_altitude() {
        assert_eq!(AIRBORNE_GS_KT, 50);
        assert!(!is_airborne(49));
        assert!(is_airborne(50));
        let (nav, ap) = (NavData::default(), airports());
        let inp = still_air(&nav, &ap);
        let airborne = |alt: i64, gs: i64| {
            let p = pilot([35.0, -78.0], alt, gs, plan("KJFK", "KMIA", ""));
            trajectory(&inp, Flight::Pilot(&p), None, now())
                .unwrap()
                .airborne()
        };
        assert!(airborne(0, 55), "55 kt at 0 ft is rolling, so airborne");
        assert!(
            airborne(250, 55),
            "the old `alt > 300` term called this ground"
        );
        assert!(!airborne(30_000, 49), "an altitude alone is not airborne");
    }

    /// AC2 padding: a not-yet-airborne flight whose route does not resolve is timed over the great
    /// circle x `GROUND_ROUTE_FACTOR`; an airborne one, or a ground one on a resolved route, is not
    /// padded. FCA metering, IDST and sector demand used to time the unpadded great circle.
    #[test]
    fn only_a_ground_flight_on_an_unresolved_route_is_padded() {
        let (none, ap) = (NavData::default(), airports());
        let inp = still_air(&none, &ap);
        let gc = gc_dist(JFK[0], JFK[1], 25.79, -80.29);
        let ground = pilot(JFK, 13, 0, plan("KJFK", "KMIA", "DCT"));
        let t = trajectory(&inp, Flight::Pilot(&ground), None, now()).unwrap();
        assert_eq!(t.path().len(), 2, "positive control: nothing resolved");
        assert!(
            (t.route_nm() - gc * GROUND_ROUTE_FACTOR).abs() < 1e-6,
            "{}",
            t.route_nm()
        );
        assert_eq!(t.modeled_nm(10.0), 10.0 * GROUND_ROUTE_FACTOR);
        let pre = prefile(plan("KJFK", "KMIA", "DCT"));
        let t = trajectory(&inp, Flight::Prefile(&pre), Some(now()), now()).unwrap();
        assert!(
            (t.route_nm() - gc * GROUND_ROUTE_FACTOR).abs() < 1e-6,
            "prefile"
        );

        // An unknown departure leaves no route anchors, so the airborne flight falls back to the
        // great circle from where it is: the one airborne case with nothing resolved.
        let air = pilot(JFK, 5_000, 250, plan("ZZZZ", "KMIA", "DCT"));
        let t = trajectory(&inp, Flight::Pilot(&air), None, now()).unwrap();
        assert_eq!(
            t.path().len(),
            2,
            "positive control: airborne, nothing resolved"
        );
        assert!(
            (t.route_nm() - gc).abs() < 1e-6,
            "airborne: {}",
            t.route_nm()
        );

        let (nav, dca) = (
            NavData::load(),
            AirportDb::from([
                ("KJFK".to_string(), Airport::at(40.64, -73.78)),
                ("KDCA".to_string(), Airport::at(38.85, -77.04)),
            ]),
        );
        let inp = still_air(&nav, &dca);
        let routed = pilot(JFK, 13, 0, plan("KJFK", "KDCA", "RBV WHITE SIE"));
        let t = trajectory(&inp, Flight::Pilot(&routed), None, now()).unwrap();
        assert!(t.path().len() > 2, "positive control: the route resolved");
        assert!((t.route_nm() - path_len_nm(t.path())).abs() < 1e-9);
    }

    #[test]
    fn route_scale_pads_only_the_unresolved_ground_case() {
        assert_eq!(route_scale(false, false), GROUND_ROUTE_FACTOR);
        assert_eq!(route_scale(false, true), 1.0);
        assert_eq!(route_scale(true, false), 1.0);
        assert_eq!(route_scale(true, true), 1.0);
    }

    /// #814 AC4: with no scheduler time a ground flight is wheels-up at its filed departure time plus
    /// 10 minutes. A filed time already past, or one that does not read (blank, malformed, `0000`),
    /// counts as now. A released wheels-up replaces the proposal, for a prefile too, and an airborne
    /// flight is wheels-up now whatever it holds. now() is 2023-11-14 22:13:20Z.
    #[test]
    fn a_ground_flight_without_a_scheduler_time_leaves_at_its_filed_time_plus_ten_minutes() {
        use chrono::TimeZone;
        let (nav, ap) = (NavData::default(), airports());
        let inp = still_air(&nav, &ap);
        let at = |h, m, s| Utc.with_ymd_and_hms(2023, 11, 14, h, m, s).unwrap();
        let filed = |deptime: &str| FlightPlan {
            deptime: deptime.into(),
            ..plan("KJFK", "KMIA", "")
        };
        let cases = [
            ("2300", at(23, 10, 0)),
            ("2220", at(22, 30, 0)),
            ("2200", at(22, 23, 20)),
            ("1100", at(22, 23, 20)),
            ("", at(22, 23, 20)),
            ("0000", at(22, 23, 20)),
            ("2460", at(22, 23, 20)),
            ("23", at(22, 23, 20)),
            ("23:0", at(22, 23, 20)),
        ];
        for (deptime, wheels_up) in cases {
            let parked = pilot(JFK, 13, 0, filed(deptime));
            let t = trajectory(&inp, Flight::Pilot(&parked), None, now()).unwrap();
            assert_eq!(t.wheels_up(), wheels_up, "pilot filed {deptime:?}");
        }
        // The nearest occurrence within half a day: 0100 is 2 h 47 min after now, tomorrow.
        let parked = pilot(JFK, 13, 0, filed("0100"));
        let t = trajectory(&inp, Flight::Pilot(&parked), None, now()).unwrap();
        assert_eq!(
            t.wheels_up(),
            Utc.with_ymd_and_hms(2023, 11, 15, 1, 10, 0).unwrap()
        );

        let parked = pilot(JFK, 13, 0, filed("2300"));
        let pre = prefile(filed("2300"));
        let release = now() + Duration::minutes(30);
        for flight in [Flight::Pilot(&parked), Flight::Prefile(&pre)] {
            let t = trajectory(&inp, flight, Some(release), now()).unwrap();
            assert_eq!(t.wheels_up(), release);
            let airborne_sec = (t.eta_at_field() - release).num_seconds();
            assert!(airborne_sec > 3600, "{airborne_sec} s from wheels-up");
        }
        // A release already passed leaves now: the flight cannot have flown the minutes it sat out.
        let late = trajectory(
            &inp,
            Flight::Pilot(&parked),
            Some(now() - Duration::minutes(5)),
            now(),
        )
        .unwrap();
        assert_eq!(late.wheels_up(), now());

        let air = pilot([35.0, -78.0], 30_000, 450, filed("2300"));
        for released in [None, Some(release)] {
            let t = trajectory(&inp, Flight::Pilot(&air), released, now()).unwrap();
            assert_eq!(t.wheels_up(), now(), "airborne, released {released:?}");
        }
    }

    /// #814 AC2: no taxi allowance sits between wheels-up and the flight. A ground flight is timed
    /// from the runway at whatever wheels-up it holds, so moving that wheels-up moves its ETA by
    /// exactly as much, and its ETE from the proposed wheels-up equals its ETE from a ready time.
    #[test]
    fn a_ground_flights_ete_runs_from_its_wheels_up() {
        let (nav, ap) = (NavData::default(), airports());
        let inp = still_air(&nav, &ap);
        let parked = pilot(JFK, 13, 0, plan("KJFK", "KMIA", ""));
        let proposed = trajectory(&inp, Flight::Pilot(&parked), None, now()).unwrap();
        let ete = proposed.eta_at_field() - proposed.wheels_up();
        for ready_min in [0, 7, 45] {
            let ready = now() + Duration::minutes(ready_min);
            let t = trajectory(&inp, Flight::Pilot(&parked), Some(ready), now()).unwrap();
            assert_eq!(t.wheels_up(), ready, "ready +{ready_min} min");
            assert_eq!(t.eta_at_field(), ready + ete, "ready +{ready_min} min");
        }
    }

    /// #813: either side of `MIT_CROSSING_NM` (40). A flight 41 nm out reads its own trajectory
    /// 40 nm short of the field; one 39 nm out has no such point left and reads its descent
    /// schedule 40 nm out: 363.4 kt for the default profile (hand-worked, as in trajectory.rs).
    #[test]
    fn the_mit_speed_switches_to_the_descent_schedule_inside_forty_miles() {
        let (nav, ap) = (NavData::default(), airports());
        let inp = still_air(&nav, &ap);
        let at = |lat: f64| Pilot {
            heading: 180,
            ..pilot([lat, -80.29], 3_000, 180, plan("KJFK", "KMIA", ""))
        };
        let (out, inside) = (at(26.4729), at(26.4396));
        let out = trajectory(&inp, Flight::Pilot(&out), None, now()).unwrap();
        let inside = trajectory(&inp, Flight::Pilot(&inside), None, now()).unwrap();
        assert!((out.route_nm() - 41.0).abs() < 0.05, "{}", out.route_nm());
        assert!(
            (inside.route_nm() - 39.0).abs() < 0.05,
            "{}",
            inside.route_nm()
        );

        assert_eq!(
            out.gs_short_of_field(40.0),
            out.gs_at(out.route_nm() - 40.0)
        );
        let near = inside.gs_short_of_field(40.0);
        assert!((near - 363.4).abs() < 0.5, "{near}");
        assert!(near > inside.gs_at(0.0), "not the speed where it is now");

        // The descent ends at the arrival field's own elevation: the same flight into a field at
        // 5,000 ft is lower 40 nm out, so it reads the schedule there.
        let mut high = airports();
        high.get_mut("KMIA").unwrap().elevation_ft = 5_000.0;
        let inp = still_air(&nav, &high);
        let p = at(26.4396);
        let high_near = trajectory(&inp, Flight::Pilot(&p), None, now())
            .unwrap()
            .gs_short_of_field(40.0);
        let expected = trajectory::descent_ground_speed_at(
            40.0,
            5_000.0,
            35_000.0,
            440.0,
            &AircraftProfile::default(),
            &RouteWind::still(),
        );
        assert_eq!(high_near, expected);
        assert!((high_near - near).abs() > 1.0, "{high_near} vs {near}");

        // It reads the flight's own filed cruise: one filed at 8,000 ft and 400 kt is already
        // cruising 40 nm out (its descent from 8,000 ft is shorter than that), faster than the
        // 363.4 kt a descent from FL350 gives there.
        let low = Pilot {
            heading: 180,
            ..pilot(
                [26.4396, -80.29],
                3_000,
                180,
                FlightPlan {
                    altitude: "8000".into(),
                    cruise_tas: "400".into(),
                    ..plan("KJFK", "KMIA", "")
                },
            )
        };
        let inp = still_air(&nav, &ap);
        let low_near = trajectory(&inp, Flight::Pilot(&low), None, now())
            .unwrap()
            .gs_short_of_field(40.0);
        assert_eq!(low_near, 400.0);

        // Its own type's profile: a B738 descending at 260/220 KIAS reads that schedule 40 nm out,
        // and the same profile times its ETA.
        let b738 = AircraftProfile {
            desc_ias_hi: 260.0,
            desc_ias_lo: 220.0,
            ..AircraftProfile::default()
        };
        let typed = ProfileTable::new(
            AircraftProfile::default(),
            HashMap::new(),
            HashMap::from([("B738".to_string(), b738.clone())]),
        );
        static STILL: LazyLock<Winds> = LazyLock::new(Winds::default);
        let inp_typed = Inputs {
            nav: &nav,
            airports: &ap,
            winds: &STILL,
            profiles: &typed,
            locked: &NO_LOCKS,
        };
        let p = at(26.4396);
        let typed_traj = trajectory(&inp_typed, Flight::Pilot(&p), None, now()).unwrap();
        let typed_near = typed_traj.gs_short_of_field(40.0);
        let expected = trajectory::descent_ground_speed_at(
            40.0,
            0.0,
            35_000.0,
            440.0,
            &b738,
            &RouteWind::still(),
        );
        assert_eq!(typed_near, expected);
        assert!(typed_near < near - 1.0, "{typed_near} vs {near}");
        assert!(
            typed_traj.eta_at_field() > inside.eta_at_field(),
            "the slower B738 schedule lands later"
        );

        // And the wind along its remaining route: a uniform 40 kt on the tail of this southbound
        // flight (`v` 40 in the grid) raises it.
        let windy = Winds::test_grid(
            (24.0, -82.0),
            (5, 4),
            &[0.0, 10_000.0, 20_000.0, 40_000.0],
            now(),
            |_, _, _| (0, 40, super::super::winds::GRID_MISSING),
        );
        let profiles = ProfileTable::default();
        let inp = Inputs {
            nav: &nav,
            airports: &ap,
            winds: &windy,
            profiles: &profiles,
            locked: &NO_LOCKS,
        };
        let p = at(26.4396);
        let windy_near = trajectory(&inp, Flight::Pilot(&p), None, now())
            .unwrap()
            .gs_short_of_field(40.0);
        assert!(
            windy_near > near + 1.0,
            "a tailwind: {windy_near} vs {near}"
        );
    }

    /// AC2 filed cruise speed: the filed number, `0` when blank or junk, then the profile cap; an
    /// unfiled speed takes the altitude default, not airport flow's old 420 kt.
    #[test]
    fn an_unfiled_cruise_speed_takes_the_altitude_default() {
        assert_eq!(filed_tas("480"), 480.0);
        assert_eq!(filed_tas(" 450 "), 450.0);
        assert_eq!(filed_tas(""), 0.0);
        assert_eq!(filed_tas("N0450"), 0.0);
        let (nav, ap) = (NavData::default(), airports());
        let inp = still_air(&nav, &ap);
        let tas = |raw: &str| {
            let p = pilot(
                [35.0, -78.0],
                30_000,
                450,
                FlightPlan {
                    cruise_tas: raw.into(),
                    ..plan("KJFK", "KMIA", "")
                },
            );
            trajectory(&inp, Flight::Pilot(&p), None, now())
                .unwrap()
                .cruise_tas()
        };
        assert_eq!(tas("480"), 480.0);
        assert_eq!(tas(""), 440.0, "the FL350 altitude default");
        assert_ne!(tas(""), 420.0);
    }

    /// AC2 filed altitude: the first level of a step-climb filing, and an unreadable one timed at
    /// 35,000 ft.
    #[test]
    fn the_filed_altitude_is_read_once() {
        let (nav, ap) = (NavData::default(), airports());
        let inp = still_air(&nav, &ap);
        let cruise = |raw: &str| {
            let p = pilot(
                [35.0, -78.0],
                30_000,
                450,
                FlightPlan {
                    altitude: raw.into(),
                    ..plan("KJFK", "KMIA", "")
                },
            );
            trajectory(&inp, Flight::Pilot(&p), None, now())
                .unwrap()
                .cruise_ft()
        };
        assert_eq!(cruise("FL350/370"), 35_000.0);
        assert_eq!(cruise("24000"), 24_000.0);
        assert_eq!(cruise("VFR"), 35_000.0);
    }

    /// #814: a caller that passes no released wheels-up still times a locked flight from its lock in
    /// the inputs, a pilot and a prefile alike; only an unlocked pilot leaves at its proposal.
    #[test]
    fn the_inputs_lock_is_used_when_the_caller_passes_none() {
        let (nav, ap) = (NavData::default(), airports());
        let lock = now() + Duration::minutes(40);
        let locked = HashMap::from([("PRE1".to_string(), lock), ("GND1".to_string(), lock)]);
        let inp = Inputs {
            locked: &locked,
            ..still_air(&nav, &ap)
        };
        let parked = Pilot {
            callsign: "GND1".into(),
            ..pilot(JFK, 13, 0, plan("KJFK", "KMIA", ""))
        };
        let t = trajectory(&inp, Flight::Pilot(&parked), None, now()).unwrap();
        assert_eq!(t.wheels_up(), lock, "pilot");
        let pre = prefile(plan("KJFK", "KMIA", ""));
        let t = trajectory(&inp, Flight::Prefile(&pre), None, now()).unwrap();
        assert_eq!(t.wheels_up(), lock, "prefile");
        let free = pilot(JFK, 13, 0, plan("KJFK", "KMIA", ""));
        let t = trajectory(&inp, Flight::Pilot(&free), None, now()).unwrap();
        assert_eq!(
            t.wheels_up(),
            now() + Duration::minutes(10),
            "positive control: unlocked"
        );
    }

    /// A prefile that holds a lock is timed from its departure field, at the lock; one whose field is
    /// unknown over the nominal leg, with no path to draw and no fixes. One that holds no lock has no
    /// proposed wheels-up and is not timed at all (#814).
    #[test]
    fn a_prefile_is_timed_from_its_departure_field_only_from_a_lock() {
        let (nav, ap) = (NavData::default(), airports());
        let inp = still_air(&nav, &ap);
        let pre = prefile(plan("KJFK", "KMIA", ""));
        assert!(
            trajectory(&inp, Flight::Prefile(&pre), None, now()).is_none(),
            "no lock, no proposed wheels-up"
        );
        let lock = now() + Duration::minutes(40);
        let t = trajectory(&inp, Flight::Prefile(&pre), Some(lock), now()).unwrap();
        assert_eq!(t.path().first(), Some(&JFK));
        assert!(!t.airborne());
        assert_eq!(t.wheels_up(), lock, "positive control: timed from its lock");

        let lost = prefile(plan("KZZZ", "KMIA", ""));
        let t = trajectory(&inp, Flight::Prefile(&lost), Some(lock), now()).unwrap();
        assert!(t.path().is_empty());
        assert_eq!(t.route_nm(), NOMINAL_ROUTE_NM);
        assert!(t.fixes().is_empty());
    }

    /// An airborne flight with neither a route nor a known destination is not timed; one with no
    /// flight plan is not either.
    #[test]
    fn an_airborne_flight_going_nowhere_known_is_not_timed() {
        let (nav, ap) = (NavData::default(), airports());
        let inp = still_air(&nav, &ap);
        let lost = pilot([35.0, -78.0], 30_000, 450, plan("KJFK", "KZZZ", ""));
        assert!(trajectory(&inp, Flight::Pilot(&lost), None, now()).is_none());
        let unfiled = Pilot {
            flight_plan: None,
            ..pilot([35.0, -78.0], 30_000, 450, plan("KJFK", "KMIA", ""))
        };
        assert!(trajectory(&inp, Flight::Pilot(&unfiled), None, now()).is_none());
    }

    /// The route is resolved up front and the profile only when a time is asked for, so a caller
    /// that drops a flight on its route (an FCA crossing, a sector box) builds nothing for it.
    #[test]
    fn the_profile_is_built_only_when_a_time_is_asked_for() {
        let (nav, ap) = (NavData::default(), airports());
        let inp = still_air(&nav, &ap);
        let p = pilot([35.0, -78.0], 30_000, 450, plan("KJFK", "KMIA", ""));
        let t = trajectory(&inp, Flight::Pilot(&p), None, now()).unwrap();
        let _ = (t.path(), t.route_nm(), t.airborne());
        assert!(t.timing.get().is_none());
        let _ = t.eta_at_field();
        assert!(t.timing.get().is_some());
    }

    /// The trajectory's times and positions agree: after the flying time to a point it is at that
    /// point, and the ETA at the end of the path is the ETA at the field.
    #[test]
    fn a_trajectory_reaches_each_point_at_its_eta() {
        let (nav, ap) = (NavData::default(), airports());
        let inp = still_air(&nav, &ap);
        for flight in [
            pilot([35.0, -78.0], 30_000, 450, plan("KJFK", "KMIA", "")),
            pilot(JFK, 13, 0, plan("KJFK", "KMIA", "")),
        ] {
            let t = trajectory(&inp, Flight::Pilot(&flight), None, now()).unwrap();
            let len = path_len_nm(t.path());
            assert_eq!(t.eta_at(len), t.eta_at_field());
            for along in [0.0, len * 0.25, len * 0.5, len * 0.9] {
                let sec = (t.eta_at(along) - t.wheels_up()).num_milliseconds() as f64 / 1000.0;
                let at = t.progress_after(sec);
                assert!(
                    (at.along_nm - along).abs() < 0.5,
                    "{along}: {}",
                    at.along_nm
                );
                assert!((at.alt_ft - t.alt_at(along)).abs() < 50.0);
            }
            assert!(t.progress_after(1e9).landed);
        }
    }

    // ---- #813 AC1: no surface times a flight outside the entry point ------------------------------

    /// The calls into the model that produce a time, a speed or a position along a route. Outside
    /// `feed/predict.rs` and `feed/trajectory.rs`, only [`trajectory`] and its [`Trajectory`] make them
    /// (and the offline profile backtest, which feeds no surface).
    const TIMING_CALLS: [&str; 8] = [
        "VerticalProfile::build",
        "descent_ground_speed_at(",
        "profile_from_here(",
        "eta_along_route(",
        "arrival_eta(",
        "capped_cruise_tas(",
        ".time_between(",
        ".distance_after(",
    ];

    /// `src` with its `//` comments and every `#[cfg(test)]` item removed: the code that ships.
    fn shipped_code(src: &str) -> String {
        let code: Vec<&str> = src
            .lines()
            .map(|l| l.split("//").next().unwrap_or(""))
            .collect();
        let mut out = String::new();
        let mut i = 0;
        while i < code.len() {
            if code[i].trim() != "#[cfg(test)]" {
                out.push_str(code[i]);
                out.push('\n');
                i += 1;
                continue;
            }
            // Skip the item the attribute is on: to its `;` if that comes before any `{`, else to
            // the brace that closes the first one.
            let (mut depth, mut opened) = (0i32, false);
            i += 1;
            while i < code.len() {
                for c in code[i].chars() {
                    match c {
                        '{' => (depth, opened) = (depth + 1, true),
                        '}' => depth -= 1,
                        _ => {}
                    }
                }
                let ends = if opened {
                    depth <= 0
                } else {
                    code[i].trim_end().ends_with(';')
                };
                i += 1;
                if ends {
                    break;
                }
            }
        }
        out
    }

    /// The [`TIMING_CALLS`] `src` makes outside its comments and test items.
    fn timing_calls(src: &str) -> Vec<&'static str> {
        let code = shipped_code(src);
        TIMING_CALLS
            .into_iter()
            .filter(|call| code.contains(call))
            .collect()
    }

    /// The test-only files a `#[cfg(test)] mod name;` in `src` declares, as `name`.
    fn test_modules(src: &str) -> Vec<String> {
        let lines: Vec<&str> = src.lines().map(str::trim).collect();
        lines
            .windows(2)
            .filter(|w| w[0] == "#[cfg(test)]")
            .filter_map(|w| {
                let m = w[1].strip_prefix("pub(crate) ").unwrap_or(w[1]);
                m.strip_prefix("mod ")?
                    .strip_suffix(';')
                    .map(str::to_string)
            })
            .collect()
    }

    fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read backend/src") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    #[test]
    fn only_the_entry_point_times_a_flight() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        rust_files(&src, &mut files);
        let sources: Vec<(std::path::PathBuf, String)> = files
            .into_iter()
            .map(|p| {
                let text = std::fs::read_to_string(&p).expect("read source");
                (p, text)
            })
            .collect();
        // A test-only file declared next to its parent (`#[cfg(test)] mod x;` -> `x.rs`).
        let test_only: Vec<std::path::PathBuf> = sources
            .iter()
            .flat_map(|(p, text)| {
                let dir = p.parent().unwrap().to_path_buf();
                test_modules(text)
                    .into_iter()
                    .map(move |m| dir.join(format!("{m}.rs")))
            })
            .collect();
        let owners = [src.join("feed/predict.rs"), src.join("feed/trajectory.rs")];
        // The offline profile backtest (#828) scores candidate aircraft profiles against recorded
        // flights with the model's own functions and feeds no surface (#813 AC1).
        let backtest = src.join("profile_builder/backtest.rs");
        let mut scanned = 0;
        let mut offenders = Vec::new();
        for (path, text) in &sources {
            if *path == backtest {
                assert!(
                    !timing_calls(text).is_empty(),
                    "the backtest exemption is still needed"
                );
                continue;
            }
            if owners.contains(path) || test_only.contains(path) {
                continue;
            }
            scanned += 1;
            for call in timing_calls(text) {
                offenders.push(format!(
                    "{}: {call}",
                    path.strip_prefix(&src).unwrap().display()
                ));
            }
        }
        assert!(scanned > 100, "positive control: {scanned} files scanned");
        assert!(
            sources.iter().any(|(p, _)| *p == backtest),
            "positive control: the exempt backtest is among the scanned sources"
        );
        assert!(
            test_only.contains(&src.join("handlers/flow_prefile_candidate_tests.rs")),
            "positive control: test-only files are found"
        );
        assert!(
            offenders.is_empty(),
            "time a flight through `predict::trajectory`, not the model directly: {offenders:?}"
        );
    }

    #[test]
    fn the_guard_flags_a_timing_call_in_shipped_code_only() {
        let shipped = "fn eta() {\n    predict::arrival_eta(a, b);\n}\n";
        assert_eq!(timing_calls(shipped), vec!["arrival_eta("]);
        // Every listed call is caught, each on its own.
        for call in TIMING_CALLS {
            let src = format!("fn f() {{ let x = {call}1); }}\n");
            assert_eq!(timing_calls(&src), vec![call], "{call}");
        }
        // The descent-schedule speed the MIT conversion reads inside 40 nm is a timing call too.
        let descent = "fn f() { trajectory::descent_ground_speed_at(40.0, 0.0, a, b, p, w); }
";
        assert_eq!(timing_calls(descent), vec!["descent_ground_speed_at("]);
        // A comment, a test module and a test-only helper are not shipped code...
        let tests = "// predict::arrival_eta(a)\n#[cfg(test)]\nmod tests {\n    fn t() {\n        vp.time_between(1.0, 0.0);\n    }\n}\n#[cfg(test)]\nfn helper() { vp.distance_after(1.0, 2.0); }\n";
        assert!(timing_calls(tests).is_empty());
        // ...but code after them is.
        let after = format!("{tests}fn late() {{ trajectory::capped_cruise_tas(0.0, 1.0, p); }}\n");
        assert_eq!(timing_calls(&after), vec!["capped_cruise_tas("]);
        // A `#[cfg(test)]` declaration ending in `;` skips only itself.
        let decl = "#[cfg(test)]\nmod x;\nfn g() { predict::eta_along_route(a); }\n";
        assert_eq!(timing_calls(decl), vec!["eta_along_route("]);
        assert_eq!(test_modules(decl), vec!["x".to_string()]);
    }

    // ---- #828: `arrival_route` is `arrival_eta`'s route resolution ---------------------------
    //
    /// `arrival_eta` timed by hand from its parts: `arrival_route`, `Winds::along` over that route,
    /// and `eta_along_route` to the field. Returns the route and the composed ETA.
    fn composed(nav: &NavData, winds: &Winds, ac: &ArrivalInput) -> (ArrivalRoute, DateTime<Utc>) {
        let profile = AircraftProfile::default();
        let ap = jfk_dca();
        let r = arrival_route(nav, &ap, ac);
        let eta = eta_along_route(
            r.airborne,
            r.route_nm,
            r.route_nm,
            ac.alt_ft,
            ac.gs as f64,
            ac.cruise_ft,
            ac.cruise_tas,
            &ap,
            ac.arr,
            &profile,
            winds.along(&r.path, r.route_nm),
            now(),
        )
        .eta;
        (r, eta)
    }

    /// Airborne on a route that resolves (`RBV WHITE SIE`) under the sheared field: the composed
    /// parts give `arrival_eta`'s ETA and route length exactly, on the resolved path's length.
    #[test]
    fn arrival_eta_is_arrival_route_along_and_eta_along_route_on_a_resolved_route() {
        let nav = NavData::load();
        let shear = Winds::test_shear();
        let ac = coastal("RBV WHITE SIE");
        let (r, eta) = composed(&nav, &shear, &ac);
        assert!(r.airborne && r.path.len() > 2);
        assert_eq!(r.route_nm, path_len_nm(&r.path));
        let live = arrival_eta(
            &nav,
            &jfk_dca(),
            &shear,
            &AircraftProfile::default(),
            &ac,
            now(),
        );
        assert_eq!(eta, live.eta);
        assert_eq!(r.route_nm, live.route_nm);
    }

    /// Airborne with nothing to resolve: the unpadded great circle to the field, and the composed
    /// parts give `arrival_eta`'s ETA.
    #[test]
    fn arrival_eta_is_arrival_route_along_and_eta_along_route_when_airborne_and_unresolved() {
        let nav = NavData::default();
        let shear = Winds::test_shear();
        let ac = coastal("");
        let (r, eta) = composed(&nav, &shear, &ac);
        assert!(r.airborne);
        assert!((r.route_nm - gc_dist(40.2, -74.0, 38.85, -77.04)).abs() < 1e-9);
        assert_eq!(eta, eta_in(&nav, &shear, &ac));
    }

    /// On the ground with nothing to resolve: the great circle carries `GROUND_ROUTE_FACTOR`, and
    /// the composed parts give `arrival_eta`'s ETA.
    #[test]
    fn arrival_eta_is_arrival_route_along_and_eta_along_route_with_ground_padding() {
        assert!((1.11..1.13).contains(&GROUND_ROUTE_FACTOR));
        let nav = NavData::default();
        let shear = Winds::test_shear();
        let ac = ArrivalInput {
            pos: [40.64, -73.78],
            gs: 0,
            alt_ft: 0.0,
            ..coastal("")
        };
        let (r, eta) = composed(&nav, &shear, &ac);
        assert!(!r.airborne);
        assert_eq!(r.path, vec![[40.64, -73.78], [38.85, -77.04]]);
        let gc = gc_dist(40.64, -73.78, 38.85, -77.04);
        assert!((r.route_nm - gc * GROUND_ROUTE_FACTOR).abs() < 1e-9);
        assert_eq!(eta, eta_in(&nav, &shear, &ac));
    }

    /// A ground flight whose route resolves is timed on the resolved length, unpadded.
    #[test]
    fn arrival_route_does_not_pad_a_ground_flight_whose_route_resolves() {
        let nav = NavData::load();
        let ac = ArrivalInput {
            pos: [40.64, -73.78],
            gs: 0,
            alt_ft: 0.0,
            ..coastal("RBV WHITE SIE")
        };
        let r = arrival_route(&nav, &jfk_dca(), &ac);
        assert!(!r.airborne && r.path.len() > 2);
        assert_eq!(r.route_nm, path_len_nm(&r.path));
    }

    /// The airborne flag flips at 50 kt, the boundary `arrival_eta` times a flight from its
    /// position rather than its departure on.
    #[test]
    fn arrival_route_calls_a_flight_airborne_from_50_kt() {
        let nav = NavData::default();
        let at = |gs| arrival_route(&nav, &jfk_dca(), &ArrivalInput { gs, ..coastal("") }).airborne;
        assert!(!at(49));
        assert!(at(50));
    }

    // ---- #814: the proposed wheels-up at its edges, and a ready time as an absolute fixture ------

    /// AC4: the constant, pinned with absolute fixtures either side of now, so a changed value fails
    /// here and not only in the case table. now() is 2023-11-14 22:13:20Z.
    #[test]
    fn the_proposed_wheels_up_constant_straddles_its_documented_value() {
        use chrono::TimeZone;
        let at = |h, m, s| Utc.with_ymd_and_hms(2023, 11, 14, h, m, s).unwrap();
        for (deptime, pushback) in [("2213", at(22, 13, 20)), ("2214", at(22, 14, 0))] {
            let wheels_up = proposed_wheels_up(deptime, now());
            assert_eq!(wheels_up - pushback, Duration::minutes(10), "{deptime}");
        }
        assert!((9..11).contains(&PROPOSED_WHEELS_UP_MIN));
    }

    /// AC4 just after midnight: a filed time is its nearest occurrence within 12 hours, so `2350` at
    /// 00:10Z is last night's and has passed.
    #[test]
    fn a_filed_time_read_across_midnight_is_its_nearest_occurrence() {
        use chrono::TimeZone;
        let now = Utc.with_ymd_and_hms(2023, 11, 15, 0, 10, 0).unwrap();
        let day = |d, h, m| Utc.with_ymd_and_hms(2023, 11, d, h, m, 0).unwrap();
        assert_eq!(filed_departure("2350", now), Some(day(14, 23, 50)));
        assert_eq!(proposed_wheels_up("2350", now), day(15, 0, 20));
        assert_eq!(filed_departure("1209", now), Some(day(15, 12, 9)));
        assert_eq!(proposed_wheels_up("1209", now), day(15, 12, 19));
        assert_eq!(filed_departure("1211", now), Some(day(14, 12, 11)));
        assert_eq!(proposed_wheels_up("1211", now), day(15, 0, 20));
        // Exactly 12 hours either side reads as the stated day, not the other one.
        assert_eq!(filed_departure("1210", now), Some(day(15, 12, 10)));
        let noon = Utc.with_ymd_and_hms(2023, 11, 15, 12, 0, 0).unwrap();
        assert_eq!(filed_departure("0000", noon), None, "0000 means unset");
        assert_eq!(filed_departure("2359", noon), Some(day(15, 23, 59)));
        assert_eq!(filed_departure("0001", noon), Some(day(15, 0, 1)));
        // Only four digits read: a sign, a fifth digit or a separator is malformed; padding is not.
        for junk in ["+130", "23000", "2:30", "23 0", "-230"] {
            assert_eq!(filed_departure(junk, now), None, "{junk:?}");
        }
        assert_eq!(filed_departure(" 2350 ", now), Some(day(14, 23, 50)));
    }

    /// AC2 absolute fixture: a parked pilot holding a ready time is wheels-up at it, and its ETA is
    /// that ready time plus the airborne ETE alone: the 47.382 min `feed::flow`'s
    /// `the_nominal_300_nm_leg_flies_still_air` holds for this plan over the 300 nm nominal leg in
    /// still air. RDY is ready now, so wheels-up now and not at the proposed time 10 min later.
    #[test]
    fn a_ground_pilot_with_a_ready_time_lands_one_airborne_ete_after_it() {
        use chrono::TimeZone;
        let (nav, ap) = (NavData::default(), airports());
        let inp = still_air(&nav, &ap);
        let fp = FlightPlan {
            departure: "KJFK".into(),
            arrival: "KZZZ".into(),
            aircraft_short: "B738".into(),
            cruise_tas: "420".into(),
            ..Default::default()
        };
        let parked = pilot(JFK, 0, 0, fp);
        let at = |h, m, s, ms| {
            Utc.with_ymd_and_hms(2023, 11, 14, h, m, s).unwrap() + Duration::milliseconds(ms)
        };
        let check = |ready: Option<DateTime<Utc>>, wheels_up: DateTime<Utc>, eta: DateTime<Utc>| {
            let t = trajectory(&inp, Flight::Pilot(&parked), ready, now()).unwrap();
            assert_eq!(t.route_nm(), 300.0, "positive control: the nominal leg");
            assert_eq!(t.wheels_up(), wheels_up);
            let off = (t.eta_at_field() - eta).num_milliseconds().abs();
            assert!(off <= 1_000, "ETA {} against {eta}", t.eta_at_field());
        };
        // 47.382 min is 47 min 22.9 s.
        check(Some(at(23, 0, 0, 0)), at(23, 0, 0, 0), at(23, 47, 22, 900));
        check(Some(now()), at(22, 13, 20, 0), at(23, 0, 42, 900));
        // With no ready time, the proposed wheels-up 10 min out, then the same ETE.
        check(None, at(22, 23, 20, 0), at(23, 10, 42, 900));
    }
}
