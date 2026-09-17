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
use super::trajectory::{self, AircraftProfile};
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

/// ETA to a point `along_nm` along the route ahead of the aircraft's current position, timed with
/// the shared vertical-profile + winds model. The profile is built over the whole remaining route
/// to the destination (so descent is modeled when the target is near the field); the target sits
/// `along_nm` ahead. Airborne aircraft start from their current altitude; ground aircraft climb
/// from the surface and carry `ground_allowance_sec` (a learned per-gate/type/runway pushback+taxi
/// estimate, #164 sub-issue E — `feed::taxi_estimate::estimate` falls back to [`GROUND_TAXI_SEC`]
/// itself when data is thin, so callers always have a value to pass here). Ignored when `airborne`
/// is true. `arr_elev_ft` is the destination's field elevation (see
/// [`crate::feed::airports::field_elevation_ft`]), where the descent ends. An airborne aircraft's prediction is anchored to `observed_gs_kt` when it is established
/// at cruise ([`trajectory::VerticalProfile::anchor_to_observed_gs`]).
#[allow(clippy::too_many_arguments)]
pub fn eta_along_route(
    airborne: bool,
    route_len_nm: f64,
    along_nm: f64,
    cur_alt_ft: f64,
    observed_gs_kt: f64,
    cruise_alt_ft: f64,
    cruise_tas: f64,
    arr_elev_ft: f64,
    profile: &AircraftProfile,
    headwind: Option<f64>,
    ground_allowance_sec: f64,
    now: DateTime<Utc>,
) -> DateTime<Utc> {
    let vp = profile_from_here(
        airborne,
        route_len_nm,
        cur_alt_ft,
        observed_gs_kt,
        cruise_alt_ft,
        cruise_tas,
        arr_elev_ft,
        profile,
        headwind,
    );
    // Distances are nm-to-destination: the aircraft is at `route_len_nm`, the target `along_nm`
    // ahead of it (i.e. `route_len_nm − along_nm` from the field).
    let target_d = (route_len_nm - along_nm).max(0.0);
    let mut sec = vp.time_between(route_len_nm, target_d);
    if !airborne {
        sec += ground_allowance_sec;
    }
    now + Duration::seconds(sec as i64)
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
    arr_elev_ft: f64,
    profile: &AircraftProfile,
    headwind: Option<f64>,
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
        arr_elev_ft,
        profile,
        headwind,
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
#[allow(clippy::too_many_arguments)]
fn profile_from_here(
    airborne: bool,
    route_len_nm: f64,
    cur_alt_ft: f64,
    observed_gs_kt: f64,
    cruise_alt_ft: f64,
    cruise_tas: f64,
    arr_elev_ft: f64,
    profile: &AircraftProfile,
    headwind: Option<f64>,
) -> trajectory::VerticalProfile {
    let start_alt = if airborne { cur_alt_ft } else { 0.0 };
    let vp = trajectory::VerticalProfile::build(
        start_alt,
        route_len_nm,
        arr_elev_ft,
        cruise_alt_ft,
        cruise_tas,
        profile,
        headwind,
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

/// Predict when an aircraft reaches its destination field, and how far along its route that is.
///
/// Resolves the filed route via [`fca::route_path`] (the trimmed remaining route for airborne
/// aircraft, the full dep→arr route for ground/proposed), then times it through
/// [`eta_along_route`] with the target at the field. Falls back to a straight-line great circle
/// `pos → arr_ll` when the route won't resolve. `ground_allowance_sec` is only applied on the
/// ground branch (see [`eta_along_route`]) — an airborne caller may pass any value.
pub fn arrival_eta(
    nav: &NavData,
    airports: &AirportDb,
    winds: &Winds,
    profile: &AircraftProfile,
    ac: &ArrivalInput,
    ground_allowance_sec: f64,
    now: DateTime<Utc>,
) -> ArrivalPrediction {
    let [lat, lon] = ac.pos;
    let airborne = ac.gs >= 50;
    let (mut route_nm, headwind, enroute_resolved) = match fca::route_path(
        nav, airports, ac.dep, ac.arr, ac.route, lat, lon, ac.hdg, ac.gs,
    ) {
        // > 2 anchors ⇒ the nav engine placed at least one enroute point, so `path_len_nm` is a
        // real routing distance. Exactly 2 (just the endpoints, or pos→field for an airborne
        // aircraft with nothing ahead) is no better than the great circle.
        Some(path) => (
            path_len_nm(&path),
            winds.route_headwind(&path, ac.cruise_ft),
            path.len() > 2,
        ),
        None => (
            gc_dist(lat, lon, ac.arr_ll[0], ac.arr_ll[1]),
            winds.route_headwind(&[ac.pos, ac.arr_ll], ac.cruise_ft),
            false,
        ),
    };
    // A not-yet-airborne flight timed on an unresolved (great-circle) distance would arrive earlier
    // than any real routing allows; restore the pre-#63 padding for that case only. An airborne
    // aircraft is timed against the route the map actually draws, so it must not be padded.
    if !airborne && !enroute_resolved {
        route_nm *= GROUND_ROUTE_FACTOR;
    }
    let eta = eta_along_route(
        airborne,
        route_nm,
        route_nm,
        ac.alt_ft,
        ac.gs as f64,
        ac.cruise_ft,
        ac.cruise_tas,
        field_elevation_ft(airports, ac.arr),
        profile,
        headwind,
        ground_allowance_sec,
        now,
    );
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
        let hw = Winds::default().route_headwind(&path, ac.cruise_ft);
        let fca_eta = eta_along_route(
            true,
            route_len,
            route_len,
            ac.alt_ft,
            ac.gs as f64,
            ac.cruise_ft,
            ac.cruise_tas,
            0.0,
            &profile,
            hw,
            GROUND_TAXI_SEC,
            now(),
        );
        assert_eq!(
            pred.eta, fca_eta,
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
            0.0,
            &profile,
            None,
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
            0.0,
            &profile,
            None,
            GROUND_TAXI_SEC * 10.0,
            now(),
        );
        assert_eq!(low, high);
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
            0.0,
            &profile,
            None,
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
            0.0,
            &profile,
            None,
            GROUND_TAXI_SEC,
            now(),
        );
        // No panic, a sane (later, not wildly-off) ETA, and it matches the old flat-8-min-only
        // behavior plus the ladder's default pushback figure.
        assert!(eta > now());
        assert!(eta >= flat);
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
            0.0,
            &profile,
            None,
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
            0.0,
            &profile,
            None,
            0.0,
            now(),
        );
        let elapsed = (eta - now()).num_seconds() as f64;
        let projected = project_along_route(
            true,
            route_len,
            35_000.0,
            observed_gs,
            35_000.0,
            440.0,
            0.0,
            &profile,
            None,
            0.0,
            elapsed,
        );
        assert!(
            (projected - along_nm).abs() < 1.0,
            "projected {projected}nm ahead, expected ~{along_nm}nm"
        );
    }

    #[test]
    fn project_along_route_clamps_at_the_destination() {
        let profile = AircraftProfile::default();
        let ahead = project_along_route(
            true, 300.0, 35_000.0, 0.0, 35_000.0, 440.0, 0.0, &profile, None, 0.0, 999_999.0,
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
            0.0,
            &profile,
            None,
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
        assert_ne!(
            at(5431.0),
            sea_level,
            "the field elevation must reach the profile"
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
}
