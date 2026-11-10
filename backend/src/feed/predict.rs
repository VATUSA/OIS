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

use chrono::{DateTime, Duration, Utc};

use super::airports::{AirportDb, field_elevation_ft};
use super::fca;
use super::flow::gc_dist;
use super::nav::NavData;
use super::trajectory::{self, AircraftProfile, RouteWind};
use super::winds::Winds;

/// Taxi + spool-up allowance added to a not-yet-airborne aircraft's flight time (the profile model
/// covers the climb itself).
pub const GROUND_TAXI_SEC: f64 = 8.0 * 60.0;

/// Great-circle → flyable-route length multiplier for a ground/proposed flight whose filed route
/// won't resolve to an enroute path. Airway routings run a little longer than the direct great
/// circle; without this a bare `DCT`/blank filing would be timed on an unachievable distance and
/// its metered wheels-up / CFR time would skew early. Matches the factor `ground_estimate` applied
/// before issue #63.
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
/// airborne (`gs >= 50`).
pub struct ArrivalRoute {
    pub path: Vec<[f64; 2]>,
    /// Along-route distance to the field, nm. A ground flight whose route does not resolve past its
    /// endpoints carries [`GROUND_ROUTE_FACTOR`].
    pub route_nm: f64,
    pub airborne: bool,
}

/// Resolve the route [`arrival_eta`] times an aircraft along: the filed route via
/// [`fca::route_path`] (the trimmed remaining route for airborne aircraft, the full dep→arr route
/// for ground/proposed), else a straight-line great circle `pos → arr_ll`. Public so the offline
/// profile backtest (`profile_builder::backtest`) times a point short of the field on exactly this
/// route.
pub fn arrival_route(nav: &NavData, airports: &AirportDb, ac: &ArrivalInput) -> ArrivalRoute {
    let [lat, lon] = ac.pos;
    let airborne = ac.gs >= 50;
    let (path, mut route_nm, enroute_resolved) = match fca::route_path(
        nav, airports, ac.dep, ac.arr, ac.route, lat, lon, ac.hdg, ac.gs,
    ) {
        // > 2 anchors ⇒ the nav engine placed at least one enroute point, so `path_len_nm` is a
        // real routing distance. Exactly 2 (just the endpoints, or pos→field for an airborne
        // aircraft with nothing ahead) is no better than the great circle.
        Some(path) => {
            let len = path_len_nm(&path);
            let resolved = path.len() > 2;
            (path, len, resolved)
        }
        None => (
            vec![ac.pos, ac.arr_ll],
            gc_dist(lat, lon, ac.arr_ll[0], ac.arr_ll[1]),
            false,
        ),
    };
    // A not-yet-airborne flight timed on an unresolved (great-circle) distance would arrive earlier
    // than any real routing allows; restore the pre-#63 padding for that case only. An airborne
    // aircraft is timed against the route the map actually draws, so it must not be padded.
    if !airborne && !enroute_resolved {
        route_nm *= GROUND_ROUTE_FACTOR;
    }
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
/// an airborne caller may pass any value.
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
            GROUND_TAXI_SEC,
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
            GROUND_TAXI_SEC,
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
    /// the composed parts (taxi allowance included) give `arrival_eta`'s ETA.
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
}
