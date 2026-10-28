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
use super::flow::{self as feed_flow, GroundAllowanceBreakdown, gc_dist};
use super::nav::NavData;
use super::runway_db::RunwayDb;
use super::taxi_estimate;
use super::trajectory::{self, AircraftProfile, ProfileTable, RouteWind};
use super::vatsim::{FlightPlan, Pilot, Prefile};
use super::winds::Winds;
use crate::models::AirportGateBody;

/// Taxi + spool-up allowance added to a not-yet-airborne aircraft's flight time (the profile model
/// covers the climb itself).
pub const GROUND_TAXI_SEC: f64 = 8.0 * 60.0;

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
/// from the surface and carry `ground_allowance_sec` (a learned per-gate/type/runway pushback+taxi
/// estimate, #164 sub-issue E — `feed::taxi_estimate::estimate` falls back to [`GROUND_TAXI_SEC`]
/// itself when data is thin, so callers always have a value to pass here). Ignored when `airborne`
/// is true. The descent ends at `arr_icao`'s field elevation, resolved from `airports` by
/// [`profile_from_here`] rather than passed in (#411). An airborne aircraft's
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
    ground_allowance_sec: f64,
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
    let mut sec = vp.time_between(route_len_nm, target_d);
    if !airborne {
        sec += ground_allowance_sec;
    }
    AlongRouteEta {
        eta: now + Duration::seconds(sec as i64),
        gs_kt: vp.ground_speed_at(target_d),
        profile: vp,
    }
}

/// Inverse of [`eta_along_route`] (#226's forward prediction scrubber): the along-route distance
/// ahead of the aircraft's **current** position reached after `elapsed_sec` of flight, using the
/// same vertical-profile + winds model. Ground aircraft first burn `ground_allowance_sec` of
/// `elapsed_sec` on the ground (floored at zero) before the profile inversion begins, mirroring how
/// `eta_along_route` adds that allowance on top of the profile's own time going forward. Returns
/// `0.0` once the aircraft would already have reached the destination.
#[allow(clippy::too_many_arguments)]
pub fn project_along_route(
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
    ground_allowance_sec: f64,
    elapsed_sec: f64,
) -> f64 {
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
    let flying_sec = if airborne {
        elapsed_sec
    } else {
        (elapsed_sec - ground_allowance_sec).max(0.0)
    };
    let target_d = vp.distance_after(route_len_nm, flying_sec);
    // `target_d` is nm-to-destination; the caller wants nm-ahead-of-current-position.
    (route_len_nm - target_d).max(0.0)
}

/// The vertical profile [`eta_along_route`] and [`project_along_route`] share: airborne aircraft
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
/// for ground/proposed), else a straight-line great circle `pos → arr_ll`. The same resolution
/// [`trajectory`] makes, through [`resolve_path`] and [`route_scale`].
pub fn arrival_route(nav: &NavData, airports: &AirportDb, ac: &ArrivalInput) -> ArrivalRoute {
    let airborne = is_airborne(ac.gs);
    let (path, resolved) = resolve_path(
        nav,
        airports,
        (ac.dep, ac.arr, ac.route),
        ac.pos,
        ac.hdg,
        ac.gs,
        Some(ac.arr_ll),
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
/// field. `ground_allowance_sec` is only applied on the ground branch (see [`eta_along_route`]) —
/// an airborne caller may pass any value. No surface calls this: they all go through
/// [`trajectory`], which decides the inputs this takes as arguments.
pub fn arrival_eta(
    nav: &NavData,
    airports: &AirportDb,
    winds: &Winds,
    profile: &AircraftProfile,
    ac: &ArrivalInput,
    ground_allowance_sec: f64,
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
        ground_allowance_sec,
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
//   circle to the field; a ground or prefiled flight with neither is timed over
//   `NOMINAL_ROUTE_NM` in still air.
// - **Padding**: `GROUND_ROUTE_FACTOR` for every not-yet-airborne flight whose route does not
//   resolve past its endpoints, so no surface times a `DCT` filing on a distance no routing flies.
// - **Filed altitude** (`trajectory::filed_altitude_ft`, unknown timed at 35,000 ft) and **filed
//   cruise speed** (`filed_tas`, capped by the profile through `capped_cruise_tas`).
// - **Departure time**: a connected flight leaves now, a prefile at its filed `deptime`
//   (`proposed_etd`), and a flight with a released wheels-up at that time.
// - **Departure runway**: the assignment `jobs::departure_runway_derive_once` stored, read by the
//   handler (`GroundInputs::assigned`), which replaces the heading guess in the ground allowance.
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

/// Departure runways already assigned, keyed `(departure ICAO, callsign)` with `(runway, source)`:
/// `repos::departure_runway::assignment_map`.
pub type AssignedRunways = HashMap<(String, String), (String, String)>;

/// What the ground allowance reads: the gate catalog, the runway table, the learned taxi samples and
/// the assigned departure runways.
pub struct GroundInputs<'a> {
    pub gates: &'a HashMap<String, Vec<AirportGateBody>>,
    pub runways: &'a RunwayDb,
    pub taxi_samples: &'a HashMap<String, Vec<taxi_estimate::TaxiSample>>,
    pub assigned: &'a AssignedRunways,
}

static NO_GATES: LazyLock<HashMap<String, Vec<AirportGateBody>>> = LazyLock::new(HashMap::new);
static NO_RUNWAYS: LazyLock<RunwayDb> = LazyLock::new(RunwayDb::default);
static NO_SAMPLES: LazyLock<HashMap<String, Vec<taxi_estimate::TaxiSample>>> =
    LazyLock::new(HashMap::new);
static NO_ASSIGNMENTS: LazyLock<AssignedRunways> = LazyLock::new(HashMap::new);

impl GroundInputs<'static> {
    /// Empty tables, for a caller that never times a flight still to push: one that times airborne
    /// flights only, or ground flights only with a released wheels-up. A ground flight timed with
    /// these gets the default allowance.
    pub fn unused() -> Self {
        GroundInputs {
            gates: &NO_GATES,
            runways: &NO_RUNWAYS,
            taxi_samples: &NO_SAMPLES,
            assigned: &NO_ASSIGNMENTS,
        }
    }
}

/// Everything [`trajectory`] reads besides the flight.
pub struct Inputs<'a> {
    pub nav: &'a NavData,
    pub airports: &'a AirportDb,
    pub winds: &'a Winds,
    pub profiles: &'a ProfileTable,
    pub ground: GroundInputs<'a>,
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
/// The route is resolved when [`trajectory`] returns; the vertical profile, the winds and the ground
/// allowance are computed the first time a time or speed is asked for, so a caller that only tests
/// the route (an FCA crossing, a sector box) pays nothing for flights it drops.
pub struct Trajectory<'a> {
    inp: &'a Inputs<'a>,
    callsign: &'a str,
    fp: &'a FlightPlan,
    prefile: bool,
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
    allowance: Option<GroundAllowanceBreakdown>,
    allowance_sec: f64,
    off_block: DateTime<Utc>,
    wheels_up: DateTime<Utc>,
}

/// Resolve `flight` for timing at `now`. `released` is a locked wheels-up (an EDCT or CFR) for a
/// flight still on the ground; it replaces the departure time and the ground allowance. `None` when
/// the flight has no flight plan, or is airborne with neither a route nor a known destination.
pub fn trajectory<'a>(
    inp: &'a Inputs<'a>,
    flight: Flight<'a>,
    released: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Option<Trajectory<'a>> {
    let (callsign, fp, prefile, origin) = match flight {
        Flight::Pilot(p) => (
            p.callsign.as_str(),
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
            let fp = pf.flight_plan.as_ref()?;
            let dep = inp.airports.get(&fp.departure.to_ascii_uppercase());
            let origin = dep.map(|a| Origin {
                pos: [a.lat, a.lon],
                hdg: 0,
                gs: 0,
                alt_ft: 0.0,
            });
            (pf.callsign.as_str(), fp, true, origin)
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
            o.pos,
            o.hdg,
            o.gs,
            arr_ll,
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
        callsign,
        fp,
        prefile,
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
/// great circle to the destination.
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
    resolve_path(nav, airports, route, pos, hdg, gs, arr_ll).map(|(path, _)| path)
}

/// A not-yet-airborne pilot this close (nm) to its arrival field has arrived there; airport flow
/// labels it "arrived" and no surface times it.
pub(crate) const ARRIVED_NM: f64 = 5.0;

/// The path a flight at `pos` is timed along, and whether the nav engine placed an enroute point on
/// it (`> 2` anchors; exactly 2 is no better than the great circle). Falls back to the great circle
/// to `arr_ll` when the filed route does not resolve, except for a flight on the ground within
/// [`ARRIVED_NM`] of the field: `route_path` returns `None` for it because it has arrived, and it
/// has nowhere left to fly.
fn resolve_path(
    nav: &NavData,
    airports: &AirportDb,
    (dep, arr, route): (&str, &str, &str),
    pos: [f64; 2],
    hdg: i64,
    gs: i64,
    arr_ll: Option<[f64; 2]>,
) -> Option<(Vec<[f64; 2]>, bool)> {
    match fca::route_path(nav, airports, dep, arr, route, pos[0], pos[1], hdg, gs) {
        Some(path) => {
            let enroute = path.len() > 2;
            Some((path, enroute))
        }
        None => arr_ll
            .filter(|a| is_airborne(gs) || gc_dist(pos[0], pos[1], a[0], a[1]) >= ARRIVED_NM)
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

/// A prefile's off-block time: filed `deptime` (HHMM Z) nudged into a sane window, else 20 minutes
/// out. Never earlier than 20 minutes from now.
pub(crate) fn proposed_etd(deptime: &str, now: DateTime<Utc>) -> DateTime<Utc> {
    let floor = now + Duration::minutes(20);
    if deptime.len() == 4 && deptime.chars().all(|c| c.is_ascii_digit()) {
        let h: u32 = deptime[0..2].parse().unwrap_or(99);
        let m: u32 = deptime[2..4].parse().unwrap_or(99);
        if h < 24
            && m < 60
            && let Some(naive) = now.date_naive().and_hms_opt(h, m, 0)
        {
            let mut t = naive.and_utc();
            if t < now - Duration::hours(2) {
                t += Duration::hours(24);
            } else if t - now > Duration::hours(12) {
                t -= Duration::hours(24);
            }
            return t.max(floor);
        }
    }
    floor
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

    /// Predicted groundspeed (kt) `nm` along-route short of the field, or where the flight is now
    /// when it is closer than that.
    pub fn gs_short_of_field(&self, nm: f64) -> f64 {
        self.timing().vp.ground_speed_at(nm.min(self.route_nm))
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

    /// Pushback, start-up and taxi-out ahead of wheels-up, seconds; 0 for an airborne flight or one
    /// with a released wheels-up.
    pub fn allowance_sec(&self) -> f64 {
        self.timing().allowance_sec
    }

    /// How the ground allowance was derived, for the FCA debug view.
    pub(crate) fn ground_allowance(&self) -> Option<&GroundAllowanceBreakdown> {
        self.timing().allowance.as_ref()
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
        let sec = t.vp.time_between(self.route_nm, to_go_nm) + t.allowance_sec;
        t.off_block + Duration::seconds(sec as i64)
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
        let allowance = (!self.airborne && self.released.is_none()).then(|| self.allowance());
        let allowance_sec = allowance.as_ref().map_or(0.0, |a| a.total_sec());
        let off_block = match self.released {
            _ if self.airborne => self.now,
            Some(r) => r.max(self.now),
            None if self.prefile => proposed_etd(&fp.deptime, self.now),
            None => self.now,
        };
        Timing {
            vp,
            cruise_ft,
            cruise_tas,
            allowance,
            allowance_sec,
            off_block,
            wheels_up: off_block + Duration::milliseconds((allowance_sec * 1000.0) as i64),
        }
    }

    /// The ground allowance, matched on the live position for a connected pilot (none for a
    /// prefile) and on the assigned runway when one is stored.
    fn allowance(&self) -> GroundAllowanceBreakdown {
        let (g, fp) = (&self.inp.ground, self.fp);
        let dep = fp.departure.to_ascii_uppercase();
        let aircraft = (!fp.aircraft_short.is_empty()).then_some(fp.aircraft_short.as_str());
        let pos = self
            .origin
            .filter(|_| !self.prefile)
            .map(|o| (o.pos[0], o.pos[1], o.hdg, o.gs));
        let assigned = g
            .assigned
            .get(&(dep.clone(), self.callsign.to_string()))
            .map(|(runway, _source)| runway.as_str());
        feed_flow::resolve_ground_allowance(
            g.gates,
            g.runways,
            g.taxi_samples,
            &dep,
            aircraft,
            pos,
            assigned,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::airports::Airport;
    use crate::feed::taxi_estimate;
    use std::collections::HashMap;

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
            GROUND_TAXI_SEC,
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
        let pred = arrival_eta(
            &nav,
            &ap,
            &Winds::default(),
            &profile,
            &ac,
            GROUND_TAXI_SEC,
            now(),
        );

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
            GROUND_TAXI_SEC,
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
    fn ground_flight_carries_the_taxi_allowance() {
        let airborne = predict(&input([30.5, -80.0], 35_000.0, 440));
        let ground = predict(&input([40.64, -73.78], 0.0, 0));
        // The ground flight flies the whole route from the surface + taxi, so it's later.
        assert!(ground.eta > airborne.eta);
        assert!(ground.route_nm > airborne.route_nm);
    }

    /// #164 sub-issue E, AC #2: an airborne prediction must be completely unaffected by
    /// `ground_allowance_sec` — `runway::collect_arrivals` (runway ETE) only ever calls this
    /// airborne, so whatever it passes for the allowance must be provably inert.
    #[test]
    fn eta_along_route_ignores_ground_allowance_when_airborne() {
        let profile = AircraftProfile::default();
        let low = eta_along_route(
            true,
            300.0,
            300.0,
            35_000.0,
            0.0,
            35_000.0,
            440.0,
            &airports(),
            "KMIA",
            &profile,
            RouteWind::still(),
            0.0,
            now(),
        );
        let high = eta_along_route(
            true,
            300.0,
            300.0,
            35_000.0,
            0.0,
            35_000.0,
            440.0,
            &airports(),
            "KMIA",
            &profile,
            RouteWind::still(),
            GROUND_TAXI_SEC * 10.0,
            now(),
        );
        assert_eq!(low.eta, high.eta);
    }

    /// #164 sub-issue E, AC #3: a thin-data airport/gate — no observations at all — must still
    /// fall through `taxi_estimate`'s ladder to its flat default and produce a sane, panic-free
    /// ETA, exactly matching pre-#181 behavior for an airport with no learned data yet.
    #[test]
    fn eta_along_route_uses_default_allowance_for_thin_data() {
        let samples: Vec<taxi_estimate::TaxiSample> = Vec::new();
        let est = taxi_estimate::estimate(&samples, Some("A1"), Some("B738"), Some("27L"));
        let allowance = taxi_estimate::ground_allowance_sec(&est);
        // 300.0 mirrors taxi_estimate's own private DEFAULT_PUSHBACK_SEC — with zero samples both
        // metrics fall to their tier defaults.
        assert_eq!(allowance, GROUND_TAXI_SEC + 300.0);

        let profile = AircraftProfile::default();
        let eta = eta_along_route(
            false,
            300.0,
            300.0,
            0.0,
            0.0,
            35_000.0,
            440.0,
            &airports(),
            "KMIA",
            &profile,
            RouteWind::still(),
            allowance,
            now(),
        );
        let flat = eta_along_route(
            false,
            300.0,
            300.0,
            0.0,
            0.0,
            35_000.0,
            440.0,
            &airports(),
            "KMIA",
            &profile,
            RouteWind::still(),
            GROUND_TAXI_SEC,
            now(),
        );
        // No panic, a sane (later, not wildly-off) ETA, and it matches the old flat-8-min-only
        // behavior plus the ladder's default pushback figure.
        assert!(eta.eta > now());
        assert!(eta.eta >= flat.eta);
    }

    // ---- project_along_route: eta_along_route's inverse, for #226's forward prediction scrubber ----

    #[test]
    fn project_along_route_at_zero_elapsed_stays_put() {
        let profile = AircraftProfile::default();
        let ahead = project_along_route(
            true,
            300.0,
            35_000.0,
            0.0,
            35_000.0,
            440.0,
            &airports(),
            "KMIA",
            &profile,
            RouteWind::still(),
            GROUND_TAXI_SEC,
            0.0,
        );
        assert_eq!(ahead, 0.0);
    }

    #[test]
    fn project_along_route_agrees_with_eta_along_route() {
        // If eta_along_route says a target `along_nm` ahead is reached at ETA `now + T`, then
        // project_along_route(elapsed = T) must project the aircraft to that same `along_nm` —
        // including when both are anchored to an observed groundspeed (#313).
        let profile = AircraftProfile::default();
        let route_len = 300.0;
        let along_nm = 120.0; // 120nm ahead of current position (180nm-to-destination target)
        let observed_gs = 470.0; // at cruise and faster than the 440 kt profile → anchored
        let eta = eta_along_route(
            true,
            route_len,
            along_nm,
            35_000.0,
            observed_gs,
            35_000.0,
            440.0,
            &airports(),
            "KMIA",
            &profile,
            RouteWind::still(),
            0.0,
            now(),
        );
        let elapsed = (eta.eta - now()).num_seconds() as f64;
        let projected = project_along_route(
            true,
            route_len,
            35_000.0,
            observed_gs,
            35_000.0,
            440.0,
            &airports(),
            "KMIA",
            &profile,
            RouteWind::still(),
            0.0,
            elapsed,
        );
        assert!(
            (projected - along_nm).abs() < 1.0,
            "projected {projected}nm ahead, expected ~{along_nm}nm"
        );
    }

    /// #411: `profile_from_here` is the one place an airborne prediction is anchored to its
    /// observed groundspeed, and nothing proved that wiring — deleting the `if airborne` branch
    /// left all 527 tests green. The only test that mentioned anchoring
    /// (`project_along_route_agrees_with_eta_along_route`) cannot catch it: both of its sides run
    /// through this same helper, so removing the anchoring moves both identically.
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
                0.0,
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

    #[test]
    fn project_along_route_clamps_at_the_destination() {
        let profile = AircraftProfile::default();
        let ahead = project_along_route(
            true,
            300.0,
            35_000.0,
            0.0,
            35_000.0,
            440.0,
            &airports(),
            "KMIA",
            &profile,
            RouteWind::still(),
            0.0,
            999_999.0,
        );
        assert_eq!(ahead, 300.0, "never projects past the destination itself");
    }

    #[test]
    fn project_along_route_burns_ground_allowance_before_moving() {
        let profile = AircraftProfile::default();
        // Elapsed time under the ground allowance: still on the ground, no distance covered.
        let ahead = project_along_route(
            false,
            300.0,
            0.0,
            0.0,
            35_000.0,
            440.0,
            &airports(),
            "KMIA",
            &profile,
            RouteWind::still(),
            GROUND_TAXI_SEC,
            60.0,
        );
        assert_eq!(ahead, 0.0);
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
                GROUND_TAXI_SEC,
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
            GROUND_TAXI_SEC,
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
            0.0,
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
            0.0,
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
            0.0,
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
        arrival_eta(nav, &jfk_dca(), winds, &profile, ac, GROUND_TAXI_SEC, now()).eta
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
            GROUND_TAXI_SEC,
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
        let eta = |w: &Winds| {
            arrival_eta(
                &nav,
                &ap,
                w,
                &AircraftProfile::default(),
                &ac,
                GROUND_TAXI_SEC,
                now(),
            )
            .eta
        };
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
            GROUND_TAXI_SEC,
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
                GROUND_TAXI_SEC,
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
            GROUND_TAXI_SEC,
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
            0.0,
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

    /// Inputs over `nav` and [`airports`], still air, the default profiles and these ground tables.
    fn with_ground<'a>(
        nav: &'a NavData,
        ap: &'a AirportDb,
        ground: GroundInputs<'a>,
    ) -> Inputs<'a> {
        static WINDS: LazyLock<Winds> = LazyLock::new(Winds::default);
        static PROFILES: LazyLock<ProfileTable> = LazyLock::new(ProfileTable::default);
        Inputs {
            nav,
            airports: ap,
            winds: &WINDS,
            profiles: &PROFILES,
            ground,
        }
    }

    const JFK: [f64; 2] = [40.64, -73.78];

    /// A route that does not resolve falls back to the great circle to the destination, except for a
    /// pilot on the ground at its arrival field, which has arrived: `route_path`'s `None` there means
    /// nothing is left to fly, and a great circle would time it as a fresh departure.
    #[test]
    fn a_landed_pilot_gets_no_great_circle_to_its_own_field() {
        let (nav, ap) = (NavData::default(), airports());
        let path = |pos: [f64; 2], gs: i64| {
            let fp = plan("KZZZ", "KMIA", "DCT");
            flight_path(&nav, &ap, &fp, pos, 190, gs)
        };
        assert_eq!(path([25.80, -80.29], 0), None, "parked at KMIA");
        assert_eq!(
            path([25.80, -80.29 + ARRIVED_NM / 60.0 * 1.2], 0),
            Some(vec![
                [25.80, -80.29 + ARRIVED_NM / 60.0 * 1.2],
                [25.79, -80.29]
            ]),
            "just outside the arrived radius, still on the ground"
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
        let inp = with_ground(&nav, &ap, GroundInputs::unused());
        let landed = pilot([25.80, -80.29], 0, 0, plan("KZZZ", "KMIA", "DCT"));
        let traj = trajectory(&inp, Flight::Pilot(&landed), None, now()).unwrap();
        assert!(traj.path().is_empty(), "nothing to cross");
    }

    /// AC2 airborne: from 50 kt, whatever the altitude, and the altitude alone never makes a flight
    /// airborne. 50-60 kt was ground to airport flow and projected traffic, airborne to FCA.
    #[test]
    fn a_flight_is_airborne_from_50_kt_at_any_altitude() {
        assert_eq!(AIRBORNE_GS_KT, 50);
        assert!(!is_airborne(49));
        assert!(is_airborne(50));
        let (nav, ap) = (NavData::default(), airports());
        let inp = with_ground(&nav, &ap, GroundInputs::unused());
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
        let inp = with_ground(&none, &ap, GroundInputs::unused());
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
        let t = trajectory(&inp, Flight::Prefile(&pre), None, now()).unwrap();
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
        let inp = with_ground(&nav, &dca, GroundInputs::unused());
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

    /// AC2 departure time: a connected ground flight leaves now, a prefile at its filed `deptime`
    /// (never sooner than 20 minutes out), and a flight with a released wheels-up at that time, with
    /// no ground allowance on top. An airborne flight ignores a release.
    #[test]
    fn each_kind_of_flight_leaves_at_its_own_time() {
        use chrono::TimeZone;
        let (nav, ap) = (NavData::default(), airports());
        let inp = with_ground(&nav, &ap, GroundInputs::unused());
        let default_allowance = 180.0 + 120.0 + GROUND_TAXI_SEC;
        let parked = pilot(JFK, 13, 0, plan("KJFK", "KMIA", ""));
        let t = trajectory(&inp, Flight::Pilot(&parked), None, now()).unwrap();
        assert_eq!(t.allowance_sec(), default_allowance);
        assert_eq!(
            t.wheels_up(),
            now() + Duration::seconds(default_allowance as i64)
        );

        // now() is 2023-11-14 22:13:20Z.
        let pre = prefile(FlightPlan {
            deptime: "2300".into(),
            ..plan("KJFK", "KMIA", "")
        });
        let t = trajectory(&inp, Flight::Prefile(&pre), None, now()).unwrap();
        let filed = Utc.with_ymd_and_hms(2023, 11, 14, 23, 0, 0).unwrap();
        assert_eq!(
            t.wheels_up(),
            filed + Duration::seconds(default_allowance as i64)
        );

        let release = now() + Duration::minutes(30);
        for flight in [Flight::Pilot(&parked), Flight::Prefile(&pre)] {
            let t = trajectory(&inp, flight, Some(release), now()).unwrap();
            assert_eq!(t.allowance_sec(), 0.0);
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

        let air = pilot([35.0, -78.0], 30_000, 450, plan("KJFK", "KMIA", ""));
        for released in [None, Some(release)] {
            let t = trajectory(&inp, Flight::Pilot(&air), released, now()).unwrap();
            assert_eq!(t.wheels_up(), now(), "airborne, released {released:?}");
            assert_eq!(t.allowance_sec(), 0.0, "airborne, released {released:?}");
        }
    }

    /// AC2 assigned runway: a stored assignment replaces the heading guess in the ground allowance,
    /// so a parked departure reaches its runway's taxi tier. FCA metering used to pass none.
    #[test]
    fn an_assigned_runway_sets_the_taxi_allowance() {
        let sample = |runway: &str, taxi_sec: i32| taxi_estimate::TaxiSample {
            gate_id: None,
            aircraft: None,
            runway: Some(runway.into()),
            pushback_sec: Some(50),
            startup_sec: Some(40),
            taxi_sec,
        };
        let mut at_jfk: Vec<_> = (0..5).map(|_| sample("04L", 900)).collect();
        at_jfk.extend((0..5).map(|_| sample("31L", 300)));
        let samples = HashMap::from([("KJFK".to_string(), at_jfk)]);
        let assigned = AssignedRunways::from([(
            ("KJFK".to_string(), "AAL1".to_string()),
            ("04L".to_string(), "auto".to_string()),
        )]);
        let (nav, ap) = (NavData::default(), airports());
        let ground = |assigned| GroundInputs {
            taxi_samples: &samples,
            assigned,
            ..GroundInputs::unused()
        };
        let parked = pilot(JFK, 13, 0, plan("KJFK", "KMIA", ""));
        let with = with_ground(&nav, &ap, ground(&assigned));
        let t = trajectory(&with, Flight::Pilot(&parked), None, now()).unwrap();
        assert_eq!(t.allowance_sec(), 50.0 + 40.0 + 900.0);
        assert_eq!(t.ground_allowance().unwrap().runway.as_deref(), Some("04L"));

        let empty = AssignedRunways::new();
        let without = with_ground(&nav, &ap, ground(&empty));
        let t = trajectory(&without, Flight::Pilot(&parked), None, now()).unwrap();
        assert_eq!(
            t.ground_allowance().unwrap().runway,
            None,
            "positive control"
        );
        assert_ne!(t.allowance_sec(), 990.0);
        // Another callsign's assignment is not this flight's.
        let other = AssignedRunways::from([(
            ("KJFK".to_string(), "DAL2".to_string()),
            ("04L".to_string(), "auto".to_string()),
        )]);
        let inp = with_ground(&nav, &ap, ground(&other));
        let t = trajectory(&inp, Flight::Pilot(&parked), None, now()).unwrap();
        assert_eq!(t.ground_allowance().unwrap().runway, None);
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
        let inp = with_ground(&nav, &ap, GroundInputs::unused());
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
        let inp = with_ground(&nav, &ap, GroundInputs::unused());
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

    /// A prefile is timed from its departure field; one whose field is unknown over the nominal leg,
    /// with no path to draw and no fixes.
    #[test]
    fn a_prefile_is_timed_from_its_departure_field() {
        let (nav, ap) = (NavData::default(), airports());
        let inp = with_ground(&nav, &ap, GroundInputs::unused());
        let pre = prefile(plan("KJFK", "KMIA", ""));
        let t = trajectory(&inp, Flight::Prefile(&pre), None, now()).unwrap();
        assert_eq!(t.path().first(), Some(&JFK));
        assert!(!t.airborne());

        let lost = prefile(plan("KZZZ", "KMIA", ""));
        let t = trajectory(&inp, Flight::Prefile(&lost), None, now()).unwrap();
        assert!(t.path().is_empty());
        assert_eq!(t.route_nm(), NOMINAL_ROUTE_NM);
        assert!(t.fixes().is_empty());
    }

    /// An airborne flight with neither a route nor a known destination is not timed; one with no
    /// flight plan is not either.
    #[test]
    fn an_airborne_flight_going_nowhere_known_is_not_timed() {
        let (nav, ap) = (NavData::default(), airports());
        let inp = with_ground(&nav, &ap, GroundInputs::unused());
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
        let inp = with_ground(&nav, &ap, GroundInputs::unused());
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
        let inp = with_ground(&nav, &ap, GroundInputs::unused());
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
    /// `feed/predict.rs` and `feed/trajectory.rs`, only [`trajectory`] and its [`Trajectory`] make them.
    const TIMING_CALLS: [&str; 8] = [
        "VerticalProfile::build",
        "profile_from_here(",
        "eta_along_route(",
        "project_along_route(",
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
        let mut scanned = 0;
        let mut offenders = Vec::new();
        for (path, text) in &sources {
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
}
