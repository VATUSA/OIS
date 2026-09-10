//! The single arrival/crossing prediction service. Every surface that estimates when a connected
//! aircraft reaches a point — the arrival ladder, airport-flow demand, runway ETE, and FCA
//! metering — resolves the filed route **once** here and times position-along-route through the
//! one [`trajectory::VerticalProfile`] model. That keeps a given aircraft's ETA identical on every
//! surface and consistent with the real route drawn on the map (issue #63): before this, the
//! arrival surfaces timed aircraft on a straight-line great circle to the field while FCA metering
//! (correctly) used the real, longer, dog-legged route.
//!
//! Route resolution ([`fca::route_path`] → the nav engine) is the same one FCA metering uses. When
//! a route can't be resolved to ≥ 2 anchors (blank/garbage route, unknown airport), we fall back to
//! a straight-line great circle to the field so an arrival is never dropped.

use chrono::{DateTime, Duration, Utc};

use super::airports::AirportDb;
use super::fca;
use super::flow::gc_dist;
use super::nav::NavData;
use super::trajectory::{self, AircraftProfile};
use super::winds::Winds;

/// Taxi + spool-up allowance added to a not-yet-airborne aircraft's flight time (the profile model
/// covers the climb itself).
pub const GROUND_TAXI_SEC: f64 = 8.0 * 60.0;

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
/// from the surface and carry a taxi allowance.
#[allow(clippy::too_many_arguments)]
pub fn eta_along_route(
    airborne: bool,
    route_len_nm: f64,
    along_nm: f64,
    cur_alt_ft: f64,
    cruise_alt_ft: f64,
    cruise_tas: f64,
    profile: &AircraftProfile,
    headwind: Option<f64>,
    now: DateTime<Utc>,
) -> DateTime<Utc> {
    let start_alt = if airborne { cur_alt_ft } else { 0.0 };
    let vp = trajectory::VerticalProfile::build(
        start_alt,
        route_len_nm,
        0.0, // arrival field elevation ≈ sea level (v1 approximation)
        cruise_alt_ft,
        cruise_tas,
        profile,
        headwind,
    );
    // Distances are nm-to-destination: the aircraft is at `route_len_nm`, the target `along_nm`
    // ahead of it (i.e. `route_len_nm − along_nm` from the field).
    let target_d = (route_len_nm - along_nm).max(0.0);
    let mut sec = vp.time_between(route_len_nm, target_d);
    if !airborne {
        sec += GROUND_TAXI_SEC;
    }
    now + Duration::seconds(sec as i64)
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
/// `pos → arr_ll` when the route won't resolve.
pub fn arrival_eta(
    nav: &NavData,
    airports: &AirportDb,
    winds: &Winds,
    profile: &AircraftProfile,
    ac: &ArrivalInput,
    now: DateTime<Utc>,
) -> ArrivalPrediction {
    let [lat, lon] = ac.pos;
    let airborne = ac.gs >= 50;
    let (route_nm, headwind) = match fca::route_path(
        nav, airports, ac.dep, ac.arr, ac.route, lat, lon, ac.hdg, ac.gs,
    ) {
        Some(path) => (
            path_len_nm(&path),
            winds.route_headwind(&path, ac.cruise_ft),
        ),
        None => (
            gc_dist(lat, lon, ac.arr_ll[0], ac.arr_ll[1]),
            winds.route_headwind(&[ac.pos, ac.arr_ll], ac.cruise_ft),
        ),
    };
    let eta = eta_along_route(
        airborne,
        route_nm,
        route_nm,
        ac.alt_ft,
        ac.cruise_ft,
        ac.cruise_tas,
        profile,
        headwind,
        now,
    );
    ArrivalPrediction { eta, route_nm }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn airports() -> AirportDb {
        HashMap::from([
            ("KJFK".to_string(), (40.64, -73.78)),
            ("KMIA".to_string(), (25.79, -80.29)),
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
    /// (FCA) path. `arrival_eta` and a crossing timed at the field are the identical code path —
    /// this locks that so the surfaces can't drift apart again.
    #[test]
    fn arrival_and_crossing_eta_agree_at_the_field() {
        // Airborne B738 ~300 nm north of KMIA, inbound from KJFK.
        let ac = input([30.5, -80.0], 35_000.0, 440);
        let pred = predict(&ac);

        // The FCA path: same resolved route, crossing point at the field (along == route_nm).
        let hw = Winds::default().route_headwind(&[ac.pos, ac.arr_ll], ac.cruise_ft);
        let crossing = eta_along_route(
            true,
            pred.route_nm,
            pred.route_nm,
            ac.alt_ft,
            ac.cruise_ft,
            ac.cruise_tas,
            &AircraftProfile::default(),
            hw,
            now(),
        );
        assert_eq!(pred.eta, crossing);
        assert!(
            pred.route_nm > 250.0 && pred.route_nm < 360.0,
            "route_nm {}",
            pred.route_nm
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
}
