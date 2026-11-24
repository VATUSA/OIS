//! #985: the forward march, against literals worked by hand.
//!
//! FLAT, the ERAM record every case flies unless it says otherwise, has the same values at every
//! altitude (tables to 60,000 ft), so each literal is exact arithmetic:
//!   climb 2,000 fpm at 300 kt   -> a 15 s step is 500 ft and 1.25 nm; climb distance to 30,000 ft 75 nm
//!   descent 2,000 fpm at 240 kt -> a 15 s step is 1.0 nm; descent distance from 30,000 ft 60 nm
//!   envelope 400 to 500 kt, filed 480 -> a 15 s cruise step is 2.0 nm
//! Cruise 30,000 ft into a sea-level field in still air: the 1,000 ft / 3 nm path reaches 30,000 ft at
//! 90 nm, and 60 + 10 = 70 < 90, so descent_start = 90. LONG is FLAT with descent at 1,000 fpm: descent
//! distance 120 nm, descent_start = 130.
//!
//! Starts are 0.5 nm off the step grid (200.5 nm, 300.25 nm) so no step ends exactly on descent_start
//! and a rounding in 30,000 / (1000 / 3) cannot move a case.
//!
//! What the literals assume; each is design text or an open question in the report:
//!   (a) only the last step is partial: a cruise step that starts above descent_start runs its 15 s
//!   (b) cruise holds the altitude it was captured at; it is not snapped to the filed level
//!   (c) TAS, wind and phase are read at the step's start
//!   (d) the 60 kt floor is applied after the AC 8 scale, as on next
//! Totals allow 0.1 s. "Departure" is the straight-line distance from the departure airport.
//!
//! Coverage matrix (VATUSA/OIS#985)
//!
//! Requirement                          | Test
//! -------------------------------------|---------------------------------------------------------------------------
//! AC 1 cruise, then descent            | a_cruising_flight_holds_level_to_descent_start_then_flies_the_path        | OK
//! AC 1 climb, cruise, descent          | a_ground_departure_climbs_cruises_and_descends                            | OK
//! AC 1 500 ft band, both sides         | a_flight_500_ft_low_cruises_and_one_501_ft_low_climbs                     | OK
//! AC 1 above its cruise altitude       | a_flight_above_its_filed_altitude_holds_it_until_the_path_is_below_it     | OK
//! AC 1 decided each step               | an_airborne_climb_becomes_a_cruise_when_it_reaches_its_level              | OK
//! AC 2 nearer departure, both sides    | an_airborne_flight_inside_top_of_descent_climbs_only_nearer_its_departure | OK
//! AC 2 departure unknown               | a_flight_inside_top_of_descent_with_no_known_departure_does_not_climb     | OK
//! AC 2 ground, short trip              | a_short_trip_climbs_until_it_meets_the_descent_path                       | OK
//! AC 2 never climbs again              | a_short_trip_climbs_once_and_then_only_descends                           | OK
//! AC 2 reaches its level first         | a_short_trip_that_reaches_its_level_holds_it_to_the_path                  | OK
//! AC 3 path to the field's elevation   | the_descent_path_ends_at_the_arrival_fields_elevation                     | OK
//! AC 3 larger of the two distances     | descent_speeds_start_at_the_larger_of_the_type_and_path_distances         | OK
//! AC 3 the 10 nm margin                | the_descent_margin_is_ten_miles                                           | OK
//! AC 3 snaps down when above           | a_flight_above_the_path_is_on_it_at_the_next_step                         | OK
//! AC 3 descent never above cruise      | no_descent_is_flown_above_the_cruise_speed                                | OK
//! AC 3 slow type, row                  | a_c208_like_row_flies_a_flat_cruise_leg                                   | OK
//! AC 3 slow type, ERAM                 | the_bundled_c208_flies_a_flat_cruise_leg_and_never_descends_faster        | OK
//! AC 3 the path is geometric           | neither_a_rows_descent_rate_nor_the_wind_moves_the_descent_path           | OK
//! AC 4 exact inverses                  | distance_after_is_the_exact_inverse_of_time_between                       | OK
//! AC 4 monotonic, additive             | elapsed_time_is_monotonic_and_additive_along_the_route                    | OK
//! AC 4 speed agrees with time          | ground_speed_at_is_the_speed_time_between_flew                            | OK
//! AC 4 altitude agrees with distance   | alt_at_is_linear_between_the_steps_it_was_flown_on                        | OK
//! AC 4 empty and sub-step routes       | a_zero_length_route_and_one_shorter_than_a_step_are_timed                 | OK
//! AC 4 groundspeed limits              | the_groundspeed_limits_hold_after_the_scale                               | OK
//! AC 4 mean headwind                   | mean_headwind_is_weighted_by_distance                                     | OK
//! AC 4 speed inside 40 nm              | descent_ground_speed_at_reads_the_path_and_descent_speeds                 | OK
//! AC 5 no ERAM ceiling                 | an_eram_type_climbs_past_where_a_row_is_capped                            | OK
//! AC 7 warm and cold air               | warm_and_cold_air_scale_the_climb_and_descent_not_the_cruise              | OK
//! AC 8 scale                           | an_established_flight_is_scaled_to_its_observed_groundspeed               | OK
//! AC 8 clamp, both sides               | the_anchor_is_held_to_fifteen_percent_either_way                          | OK
//! AC 8 gate                            | only_a_flight_within_500_ft_of_its_level_or_above_it_is_scaled            | OK
//! AC 8 no 250 kt gate                  | a_slow_flight_at_cruise_is_scaled                                         | OK
//! AC 8, 9 ground                       | a_ground_flight_is_never_scaled_or_capped                                 | OK
//! AC 8 wind along the heading          | the_anchor_takes_the_headwind_it_is_given_not_the_routes                  | OK
//! AC 8 level flight, at its level      | a_stepped_down_flight_is_scaled_at_its_present_level                      | OK
//! AC 9 15 kt deadband, both sides      | a_descent_slower_by_more_than_15_kt_cas_is_capped_at_its_observed_speed   | OK
//! AC 9 CAS, to the end                 | the_assigned_speed_is_a_cas_held_to_the_field                             | OK
//! AC 9 not also scaled                 | a_capped_flight_is_not_also_scaled                                        | OK
//! AC 9 against the scaled prediction   | a_flight_matching_its_scaled_descent_keeps_the_scale                      | OK
//! AC 9 faster changes nothing          | a_faster_descent_changes_nothing                                          | OK
//! AC 9 wind along the heading          | the_observed_cas_adds_the_headwind_along_the_heading                      | OK
//! AC 9 not a climbing flight           | a_short_trip_still_climbing_is_not_capped                                 | OK
//! AC 9 not outside top of descent      | a_slow_climb_outside_top_of_descent_is_not_capped                         | OK
//! AC 10 1.5x climb distance, both sides | a_flight_past_one_and_a_half_climb_distances_is_level_where_it_is        | OK
//! AC 10 departure unknown              | a_low_flight_with_no_known_departure_climbs                               | OK
//! AC 10 only outside top of descent    | a_low_flight_inside_top_of_descent_is_not_made_level                      | OK
//! AC 10 path from the filed altitude   | a_level_flights_top_of_descent_is_still_its_filed_altitudes               | OK

use super::super::performance::{EramRecord, EramTable};
use super::*;

/// A record with one value at every altitude: climb 2,000 fpm at `climb_kt`, descent `(fpm, kt)`, cruise
/// between `(min, max)`.
fn record(climb_kt: f64, descent: (f64, f64), envelope: (f64, f64)) -> EramRecord {
    EramRecord::for_test(
        vec![[0.0, 2_000.0, climb_kt], [60_000.0, 2_000.0, climb_kt]],
        vec![
            [0.0, descent.0, descent.1],
            [60_000.0, descent.0, descent.1],
        ],
        vec![[0.0, envelope.0]],
        vec![[0.0, envelope.1]],
    )
}

/// FLAT, cruising 300 to 400 kt at 20,000 ft and 400 to 500 kt at 30,000 ft.
fn sloped_envelope() -> EramRecord {
    EramRecord::for_test(
        vec![[0.0, 2_000.0, 300.0]],
        vec![[0.0, 2_000.0, 240.0]],
        vec![[20_000.0, 300.0], [30_000.0, 400.0]],
        vec![[20_000.0, 400.0], [30_000.0, 500.0]],
    )
}

/// One flight to fly: FLAT, filed 30,000 ft at 480 kt into a sea-level field in still air.
struct Case {
    alt: f64,
    d: f64,
    /// Groundspeed and the headwind along the heading; None on the ground.
    gs: Option<(f64, Option<f64>)>,
    dep: Option<f64>,
    cruise: f64,
    tas: f64,
    elev: f64,
    rec: EramRecord,
    wind: RouteWind,
}

fn ground(d: f64) -> Case {
    Case {
        alt: 0.0,
        d,
        gs: None,
        dep: None,
        cruise: 30_000.0,
        tas: 480.0,
        elev: 0.0,
        rec: record(300.0, (2_000.0, 240.0), (400.0, 500.0)),
        wind: RouteWind::still(),
    }
}

/// Airborne at `alt`, `d` from the field, `dep` from its departure.
fn air(alt: f64, d: f64, gs: f64, dep: Option<f64>) -> Case {
    Case {
        alt,
        gs: Some((gs, None)),
        dep,
        ..ground(d)
    }
}

/// LONG: FLAT with a 1,000 fpm descent, at cruise 200.5 nm out.
fn long() -> Case {
    Case {
        rec: record(300.0, (1_000.0, 240.0), (400.0, 500.0)),
        ..air(30_000.0, 200.5, 480.0, Some(300.0))
    }
}

impl Case {
    fn fly(&self) -> VerticalProfile {
        let start = FlightStart {
            alt_ft: self.alt,
            to_go_nm: self.d,
            airborne: self.gs.map(|(gs_kt, headwind_kt)| Airborne {
                gs_kt,
                headwind_kt,
                from_departure_nm: self.dep,
            }),
        };
        let filed = Filed {
            cruise_ft: self.cruise,
            tas_kt: self.tas,
            arr_elev_ft: self.elev,
        };
        VerticalProfile::build(
            &start,
            &filed,
            Performance::Eram(&self.rec),
            self.wind.clone(),
        )
    }
}

/// A flight on a row with no groundspeed read.
fn build_row(
    p: &AircraftProfile,
    alt_ft: f64,
    to_go_nm: f64,
    cruise_ft: f64,
    tas_kt: f64,
    wind: RouteWind,
) -> VerticalProfile {
    let start = FlightStart {
        alt_ft,
        to_go_nm,
        airborne: None,
    };
    let filed = Filed {
        cruise_ft,
        tas_kt,
        arr_elev_ft: 0.0,
    };
    VerticalProfile::build(&start, &filed, Performance::Row(p), wind)
}

/// Calm air at `ratio` times the standard temperature up to 30,000 ft.
fn air_at(ratio: f64) -> RouteWind {
    RouteWind::new(vec![WindKnot {
        d_nm: 0.0,
        levels: [0.0, 30_000.0]
            .into_iter()
            .map(|alt| (alt, 0.0, Some(isa_temp_k(alt) * ratio)))
            .collect(),
    }])
}

/// The whole flight takes `want` seconds.
fn secs(vp: &VerticalProfile, want: f64) {
    let got = vp.time_between(1e9, 0.0);
    assert!((got - want).abs() <= 0.1, "flew {got} s, expected {want}");
}

/// The altitude at each distance, within a foot.
fn alts(vp: &VerticalProfile, at: &[(f64, f64)]) {
    for &(d, want) in at {
        let got = vp.alt_at(d);
        assert!(
            (got - want).abs() <= 1.0,
            "{d} nm: {got} ft, expected {want}"
        );
    }
}

fn peak(vp: &VerticalProfile, d: f64) -> f64 {
    (0..=(d * 10.0) as usize)
        .map(|i| vp.alt_at(i as f64 / 10.0))
        .fold(0.0, f64::max)
}

// AC 1. FLAT, airborne 30,000 ft, 200.5 nm, gs 480, departure 300.
// Cruise: 56 steps x 2 nm -> 88.5 nm, 840 s. Descent: 88.5 nm x 15 s = 1,327.5 s. Total 2,167.5 s.
// gs 480 at 200 and 91; 240 at 60.25 and 30.25. Altitude 30,000 at 91; 20,083.3 at 60.25; 10,083.3 at
// 30.25; 0 at 0. Red when: the descent phase is keyed on altitude instead of distance, or never starts.
#[test]
fn a_cruising_flight_holds_level_to_descent_start_then_flies_the_path() {
    let vp = air(30_000.0, 200.5, 480.0, Some(300.0)).fly();
    secs(&vp, 2_167.5);
    for (d, gs) in [
        (200.0, 480.0),
        (91.0, 480.0),
        (60.25, 240.0),
        (30.25, 240.0),
    ] {
        assert_eq!(vp.ground_speed_at(d), gs, "{d}");
    }
    alts(
        &vp,
        &[
            (91.0, 30_000.0),
            (60.25, 20_083.3),
            (30.25, 10_083.3),
            (0.0, 0.0),
        ],
    );
}

// AC 1. FLAT, ground, 300.25 nm, filed 30,000 / 480. Climb: 59 steps -> 29,500 ft, 73.75 nm, 885 s, at
// 226.5 nm. Cruise: 69 steps x 2 nm -> 88.5 nm, 1,035 s. Descent 1,327.5 s. Total 3,247.5 s.
// gs 300 at 250 (altitude 20,100: 50.25 nm of climb at 400 ft/nm), 480 at 200, 240 at 30.25.
// Altitude at 200: 29,500 (assumption (b)). Red when: the climb is flown at cruise speed or never captured.
#[test]
fn a_ground_departure_climbs_cruises_and_descends() {
    let vp = ground(300.25).fly();
    secs(&vp, 3_247.5);
    for (d, gs) in [(250.0, 300.0), (200.0, 480.0), (30.25, 240.0)] {
        assert_eq!(vp.ground_speed_at(d), gs, "{d}");
    }
    alts(&vp, &[(300.25, 0.0), (250.0, 20_100.0), (200.0, 29_500.0)]);
}

// AC 1. FLAT, airborne 200.5 nm out, gs 480, departure 100 (under 1.5 x 75 = 112.5, so AC 10 is off).
// 29,500 ft: cruise at once, gs 480 at 200, total 2,167.5 s. 29,499 ft: one climb step (1.25 nm, 15 s, to
// 29,999 ft), gs 300 at 200; cruise 199.25 -> 89.25 in 55 steps (825 s); descent 89.25 x 15 = 1,338.75 s.
// Total 2,178.75 s. Red when: the band is 300 ft, 1,000 ft or exclusive.
#[test]
fn a_flight_500_ft_low_cruises_and_one_501_ft_low_climbs() {
    let at = |alt: f64| air(alt, 200.5, 480.0, Some(100.0)).fly();
    let inside = at(29_500.0);
    assert_eq!(inside.ground_speed_at(200.0), 480.0);
    secs(&inside, 2_167.5);
    let outside = at(29_499.0);
    assert_eq!(outside.ground_speed_at(200.0), 300.0);
    secs(&outside, 2_178.75);
}

// AC 1, AC 3. FLAT filed 30,000 ft, airborne at 31,000 ft, 200.5 nm. Altitude 31,000 at 150 and at 89.5; the
// first descent step, 88.5 -> 87.5, ends on the path: 29,166.7 at 87.5. Total 2,167.5 s with gs 480.
// Red when: cruise snaps it to 30,000, or the path only lowers flights already below their filing.
#[test]
fn a_flight_above_its_filed_altitude_holds_it_until_the_path_is_below_it() {
    let vp = air(31_000.0, 200.5, 480.0, Some(300.0)).fly();
    // Cruising at 31,000 ft, the path meets it at 93 nm.
    alts(
        &vp,
        &[(150.0, 31_000.0), (94.0, 31_000.0), (87.5, 29_166.7)],
    );
    secs(&vp, 840.0 - 30.0 + 92.5 * 15.0 + 0.0);
}

// AC 1. FLAT, airborne 20,000 ft, 200.5 nm, departure 111 (climbing: under 112.5), gs 420. Climb 19 steps to
// 29,500 ft (23.75 nm, 285 s) -> 176.75; cruise 44 steps -> 88.75 (660 s); descent 88.75 x 15 = 1,331.25.
// Total 2,276.25 s. Altitude 20,600 at 199; gs 300 at 199, 480 at 150. Red when: the phase is fixed at start.
#[test]
fn an_airborne_climb_becomes_a_cruise_when_it_reaches_its_level() {
    let vp = air(20_000.0, 200.5, 420.0, Some(111.0)).fly();
    secs(&vp, 2_276.25);
    alts(&vp, &[(199.0, 20_600.0)]);
    assert_eq!(vp.ground_speed_at(199.0), 300.0);
    assert_eq!(vp.ground_speed_at(150.0), 480.0);
}

// AC 2. FLAT, airborne 5,000 ft, 60 nm to go (inside 90), gs 300. Departure:
//   40 and 59.9 (nearer): climbs 400 ft/nm until the path, met in step 17 (13,500 ft against 12,916.7 at
//     38.75 nm): altitude 12,933 at 39, 10,000 at 30; 17 x 15 + 38.75 x 15 = 836.25 s
//   60 and 80 (no farther from the field than from the departure): altitude 5,000 at 39 and at 30, on the
//     path from 15 nm (3,333.3 at 10); 60 x 15 = 900 s
// Red when: the comparison is <=, is reversed, or uses the along-route distance flown.
#[test]
fn an_airborne_flight_inside_top_of_descent_climbs_only_nearer_its_departure() {
    for dep in [40.0, 59.9] {
        let vp = air(5_000.0, 60.0, 300.0, Some(dep)).fly();
        secs(&vp, 836.25);
        alts(&vp, &[(39.0, 12_933.3), (30.0, 10_000.0)]);
    }
    for dep in [60.0, 80.0] {
        let vp = air(5_000.0, 60.0, 300.0, Some(dep)).fly();
        secs(&vp, 900.0);
        alts(&vp, &[(39.0, 5_000.0), (30.0, 5_000.0), (10.0, 3_333.3)]);
    }
}

// AC 2 (design). The 60 nm flight above with no known departure: 900 s, altitude 5,000 at 30.
// Control: departure 40 gives 836.25 s. Red when: unknown is read as 0 nm (it would climb).
#[test]
fn a_flight_inside_top_of_descent_with_no_known_departure_does_not_climb() {
    let vp = air(5_000.0, 60.0, 300.0, None).fly();
    secs(&vp, 900.0);
    alts(&vp, &[(30.0, 5_000.0)]);
    secs(&air(5_000.0, 60.0, 300.0, Some(40.0)).fly(), 836.25);
}

// AC 2. FLAT, ground, 60 nm. Climb 400 ft/nm against a path of 333.3 ft/nm: they meet at 27.27 nm, so in step
// 22 (27.5 nm, 330 s). Then 32.5 nm x 15 = 487.5 s. Total 817.5 s. Altitude 4,000 at 50; peak between 10,800
// and 11,000; 10,000 at 30; 5,416.7 at 16.25. gs 300 at 50, 240 at 30.
// Red when: a ground flight inside descent_start descends at once (900 s), or climbs on to its level.
#[test]
fn a_short_trip_climbs_until_it_meets_the_descent_path() {
    let vp = ground(60.0).fly();
    secs(&vp, 817.5);
    alts(&vp, &[(50.0, 4_000.0), (30.0, 10_000.0), (16.25, 5_416.7)]);
    let peak = peak(&vp, 60.0);
    assert!((10_800.0..=11_000.0).contains(&peak), "{peak}");
    assert_eq!(vp.ground_speed_at(50.0), 300.0);
    assert_eq!(vp.ground_speed_at(30.0), 240.0);
}

// AC 2. The 60 nm ground flight and the 60 nm airborne flight with departure 40, sampled every 0.25 nm from
// start to field: the altitude rises, then never rises again (one sign change), and never exceeds the path
// by more than one climb step (500 ft). LONG from the ground over 120 nm (inside its 130) does the same: it
// meets the path 55 nm out at about 21,700 ft, total 1,635 s.
// Red when: the climbing flag is not cleared once the path is reached.
#[test]
fn a_short_trip_climbs_once_and_then_only_descends() {
    let mut long = ground(120.0);
    long.rec = record(300.0, (1_000.0, 240.0), (400.0, 500.0));
    secs(&long.fly(), 1_635.0);
    for (vp, d) in [
        (ground(60.0).fly(), 60.0),
        (air(5_000.0, 60.0, 300.0, Some(40.0)).fly(), 60.0),
        (long.fly(), 120.0),
    ] {
        let alts: Vec<f64> = (0..=(d * 4.0) as usize)
            .map(|i| vp.alt_at(d - i as f64 / 4.0))
            .collect();
        let top = (0..alts.len())
            .max_by(|&a, &b| alts[a].total_cmp(&alts[b]))
            .unwrap();
        assert!(top > 0, "it climbed");
        assert!(
            alts[..=top].windows(2).all(|w| w[1] >= w[0]),
            "climbs to its peak"
        );
        assert!(
            alts[top..].windows(2).all(|w| w[1] <= w[0]),
            "then only descends"
        );
        for (i, alt) in alts.iter().enumerate() {
            let path = (d - i as f64 / 4.0) * 1_000.0 / 3.0;
            assert!(*alt <= path + 500.0, "{alt} ft at step {i}, path {path}");
        }
    }
}

// AC 2. FLAT with descent at 500 fpm, cruise 8,000 ft (descent distance 16 min x 4 = 64, descent_start 74),
// ground, 70 nm: 20 nm out the path is at 16,667 ft, so it reaches its level first, flies it at the descent
// speed (gs 240 at 40) and leaves it on the path (6,666.7 at 20). Altitude at 40 between 7,500 and 8,000
// (open question: whether the 500 ft band ends this climb). Never above 8,000.
#[test]
fn a_short_trip_that_reaches_its_level_holds_it_to_the_path() {
    let mut f = ground(70.0);
    f.rec = record(300.0, (500.0, 240.0), (400.0, 500.0));
    f.cruise = 8_000.0;
    let vp = f.fly();
    assert_eq!(vp.ground_speed_at(40.0), 240.0);
    alts(&vp, &[(40.0, 7_500.0), (20.0, 6_666.7)]);
    assert!(peak(&vp, 70.0) <= 8_000.0);
}

// AC 3. FLAT into a 6,000 ft field, airborne 30,000 ft, 200.5 nm. Path 24,000 / 333.3 = 72 nm, type
// 48 + 10 = 58, so descent_start 72. Cruise 65 steps -> 70.5 (975 s), 70.5 x 15 = 1,057.5. Total 2,032.5 s
// (135 s before the sea-level field's 2,167.5). gs 480 at 71; altitude 16,000 at 30 and 6,000 at 0.
// Red when: the path ends at sea level, or descent_start ignores the elevation.
#[test]
fn the_descent_path_ends_at_the_arrival_fields_elevation() {
    let mut f = air(30_000.0, 200.5, 480.0, Some(300.0));
    f.elev = 6_000.0;
    let vp = f.fly();
    secs(&vp, 2_032.5);
    assert_eq!(vp.ground_speed_at(71.0), 480.0);
    alts(&vp, &[(30.0, 16_000.0), (0.0, 6_000.0)]);
}

// AC 3. LONG from 30,000 ft, 200.5 nm: descent_start 130. Cruise 36 steps -> 128.5 (540 s), 128.5 x 15 =
// 1,927.5. Total 2,467.5 s. At 100.25 nm gs 240 while still at 30,000 ft (the path is 33,417 there). FLAT is
// the other branch: path 90 > 70, gs 480 at 91, 240 at 88. Red when: max becomes min, or an argument is lost.
#[test]
fn descent_speeds_start_at_the_larger_of_the_type_and_path_distances() {
    let vp = long().fly();
    secs(&vp, 2_467.5);
    assert_eq!(vp.ground_speed_at(100.25), 240.0);
    alts(&vp, &[(100.25, 30_000.0)]);
    let path = air(30_000.0, 200.5, 480.0, Some(300.0)).fly();
    assert_eq!(path.ground_speed_at(91.0), 480.0);
    assert_eq!(path.ground_speed_at(88.0), 240.0);
}

// AC 3. LONG, type distance exactly 120 nm: gs 480 at 131 and 240 at 128, absolute fixtures either side of
// 130. Red when: the margin is 0 (480 at 128) or 20 (240 at 131).
#[test]
fn the_descent_margin_is_ten_miles() {
    let vp = long().fly();
    assert_eq!(vp.ground_speed_at(131.0), 480.0);
    assert_eq!(vp.ground_speed_at(128.0), 240.0);
}

// AC 3. FLAT, airborne 20,000 ft, 30 nm out (path 10,000), departure 300: altitude 20,000 at 30, 9,666.7 at
// 29, 9,333.3 at 28. One below the path (5,000 ft at 30) holds 5,000 until 15 nm, then 3,333.3 at 10.
// Red when: the descent is rate-limited, or a low flight is lifted to the path.
#[test]
fn a_flight_above_the_path_is_on_it_at_the_next_step() {
    let high = air(20_000.0, 30.0, 300.0, Some(300.0)).fly();
    alts(&high, &[(30.0, 20_000.0), (29.0, 9_666.7), (28.0, 9_333.3)]);
    let low = air(5_000.0, 30.0, 300.0, Some(300.0)).fly();
    alts(&low, &[(29.0, 5_000.0), (16.0, 5_000.0), (10.0, 3_333.3)]);
}

// AC 3. FLAT with descent at 520 kt: gs 480 at 40, total 200.5 nm / 480 = 1,503.75 s. Descent 460 in air at
// 1.21 x ISA (506 scaled): gs 480 at 40. The default row, filed 440, sampled every 5 nm over 300 nm: never
// above 440. Red when: the cap is dropped for either performance kind.
#[test]
fn no_descent_is_flown_above_the_cruise_speed() {
    let mut f = air(30_000.0, 200.5, 480.0, Some(300.0));
    f.rec = record(300.0, (2_000.0, 520.0), (400.0, 500.0));
    let vp = f.fly();
    assert_eq!(vp.ground_speed_at(40.0), 480.0);
    secs(&vp, 1_503.75);
    f.rec = record(300.0, (2_000.0, 460.0), (400.0, 500.0));
    f.wind = air_at(1.21);
    assert_eq!(f.fly().ground_speed_at(40.0), 480.0);

    let default = AircraftProfile::default();
    let vp = build_row(
        &default,
        35_000.0,
        300.0,
        35_000.0,
        440.0,
        RouteWind::still(),
    );
    for i in 0..=60 {
        let d = 5.0 * i as f64;
        assert!(
            vp.ground_speed_at(d) <= 440.0,
            "{d}: {}",
            vp.ground_speed_at(d)
        );
    }
}

// AC 3 (#361). A C208-like row: 100/120 KIAS, 900/700 fpm, cruise_tas 170, descent 140/110 KIAS at 800 fpm,
// cruising 10,000 ft, 200.5 nm out. Descent distance 24.7, descent_start 34.7. gs exactly 170 at 200, 150,
// 100, 60, 40 and 36; 162.9 (140 KIAS at 10,000 ft) at 33; 121.6 +/- 2 at 20.
// Red when: the cruise leg ramps toward the slower descent speed.
#[test]
fn a_c208_like_row_flies_a_flat_cruise_leg() {
    let c208 = AircraftProfile {
        climb_ias_lo: 100.0,
        climb_ias_hi: 120.0,
        climb_fpm_lo: 900.0,
        climb_fpm_hi: 700.0,
        cruise_tas: Some(170.0),
        service_ceiling_ft: 25_000.0,
        desc_ias_hi: 140.0,
        desc_ias_lo: 110.0,
        desc_fpm: 800.0,
        ..AircraftProfile::default()
    };
    let vp = build_row(&c208, 10_000.0, 200.5, 10_000.0, 170.0, RouteWind::still());
    for d in [200.0, 150.0, 100.0, 60.0, 40.0, 36.0] {
        assert_eq!(vp.ground_speed_at(d), 170.0, "{d}");
    }
    let near = |d: f64, want: f64, within: f64| {
        let gs = vp.ground_speed_at(d);
        assert!((gs - want).abs() <= within, "{d}: {gs}");
    };
    near(33.0, 162.9, 0.2);
    near(20.0, 121.6, 2.0);
}

// AC 3. The bundled C208 (descent table 188 kt at 10,000 ft, above its envelope) at 12,000 ft unfiled:
// cruise (149 + 178) / 2 = 163.5, descent_start 48.3 + 10 = 58.3, 250.5 nm out. gs 163.5 at 200, 100 and 60,
// and no sample to the field above 163.5. Red when: the faster descent table is flown uncapped.
#[test]
fn the_bundled_c208_flies_a_flat_cruise_leg_and_never_descends_faster() {
    let start = FlightStart {
        alt_ft: 12_000.0,
        to_go_nm: 250.5,
        airborne: None,
    };
    let filed = Filed {
        cruise_ft: 12_000.0,
        tas_kt: 0.0,
        arr_elev_ft: 0.0,
    };
    let perf = Performance::Eram(EramTable::bundled().get("C208").unwrap());
    let vp = VerticalProfile::build(&start, &filed, perf, RouteWind::still());
    for d in [200.0, 100.0, 60.0] {
        let gs = vp.ground_speed_at(d);
        assert!((gs - 163.5).abs() < 0.1, "{d}: {gs}");
    }
    for i in 0..=250 {
        assert!(vp.ground_speed_at(i as f64) <= 163.6, "{i}");
    }
}

// AC 3, in place of a_low_level_tailwind_moves_top_of_descent_further_out and
// a_headwind_on_the_upper_descent_moves_top_of_descent_in: the altitude is geometric. The default row at
// FL350, 300 nm, in still air, in 60 kt on the tail below 10,000 ft, in 80 kt on the nose from 60 nm out, and
// with desc_fpm 3,000: altitude 20,000 +/- 1 at 60 and 10,000 +/- 1 at 30 in all four.
// Control: the headwind case is later than still air.
#[test]
fn neither_a_rows_descent_rate_nor_the_wind_moves_the_descent_path() {
    let default = AircraftProfile::default();
    let steep = AircraftProfile {
        desc_fpm: 3_000.0,
        ..AircraftProfile::default()
    };
    let low_tail = RouteWind::new(vec![WindKnot {
        d_nm: 0.0,
        levels: vec![
            (0.0, -60.0, None),
            (9_000.0, -60.0, None),
            (11_000.0, 0.0, None),
            (35_000.0, 0.0, None),
        ],
    }]);
    let upper_head = RouteWind::new(
        [(0.0, 0.0), (50.0, 0.0), (60.0, 80.0), (300.0, 80.0)]
            .into_iter()
            .map(|(d_nm, hw)| WindKnot {
                d_nm,
                levels: vec![(0.0, hw, None)],
            })
            .collect(),
    );
    let fly = |p: &AircraftProfile, w: RouteWind| build_row(p, 35_000.0, 300.0, 35_000.0, 440.0, w);
    let still = fly(&default, RouteWind::still());
    let windy = fly(&default, upper_head);
    for vp in [
        &still,
        &fly(&default, low_tail),
        &windy,
        &fly(&steep, RouteWind::still()),
    ] {
        alts(vp, &[(60.0, 20_000.0), (30.0, 10_000.0)]);
    }
    assert!(windy.time_between(300.0, 0.0) > still.time_between(300.0, 0.0) + 60.0);
}

// AC 4. The ground departure (300.25 nm): for to_d in 299.9 (inside the first step), 263.1, 226.5 (a step
// edge), 150.3, 89.9, 45.45 and 0.3 (inside the last, partial step), and from_d in 300.25 and 180.7 (the
// to_d below it): |distance_after(from, time_between(from, to)) - to| < 1e-6, and the other way within 1e-6 s.
// Literal: time_between(300.25, 226.5) = 885.0 and distance_after(300.25, 885) = 226.5.
// Red when: distance_after bisects a different integral, or measures from the start instead of from_d.
#[test]
fn distance_after_is_the_exact_inverse_of_time_between() {
    let vp = ground(300.25).fly();
    assert!((vp.time_between(300.25, 226.5) - 885.0).abs() < 1e-6);
    assert!((vp.distance_after(300.25, 885.0) - 226.5).abs() < 1e-6);
    for from in [300.25, 180.7] {
        for to in [299.9, 263.1, 226.5, 150.3, 89.9, 45.45, 0.3] {
            if to >= from {
                continue;
            }
            let t = vp.time_between(from, to);
            let back = vp.distance_after(from, t);
            assert!((back - to).abs() < 1e-6, "{from} to {to}: {back}");
            assert!(
                (vp.time_between(from, back) - t).abs() < 1e-6,
                "{from} to {to}"
            );
        }
    }
}

// AC 4. The ground departure: every second from 0 to 3,247 the distance strictly falls; at 3,247.5 and at 1e9
// it is exactly 0.0; at 0 and at a negative time it is the start. time_between(300, 200) +
// time_between(200, 50.7) = time_between(300, 50.7) within 1e-9; swapped arguments give the same value; equal
// arguments give 0. Red when: an interval's own partial step is dropped.
#[test]
fn elapsed_time_is_monotonic_and_additive_along_the_route() {
    let vp = ground(300.25).fly();
    let mut last = vp.distance_after(300.25, 0.0);
    assert_eq!(last, 300.25);
    assert_eq!(vp.distance_after(300.25, -5.0), 300.25);
    for t in 1..=3_247 {
        let d = vp.distance_after(300.25, t as f64);
        assert!(d < last, "{t}: {d} after {last}");
        last = d;
    }
    assert_eq!(vp.distance_after(300.25, 3_247.5 + 1e-6), 0.0);
    assert_eq!(vp.distance_after(300.25, 1e9), 0.0);
    let parts = vp.time_between(300.0, 200.0) + vp.time_between(200.0, 50.7);
    assert!((parts - vp.time_between(300.0, 50.7)).abs() < 1e-9);
    assert_eq!(vp.time_between(50.7, 300.0), vp.time_between(300.0, 50.7));
    assert_eq!(vp.time_between(120.0, 120.0), 0.0);
}

// AC 4 (#361). The ground departure: for a pair strictly inside one step (250.2, 250.1 climb; 200.2, 199.9
// cruise; 45.3, 45.1 descent; 0.3, 0.1 the last step), 3600 x (a - b) / time_between(a, b) equals
// ground_speed_at of the midpoint within 1e-6, and is 300, 480, 240 and 240.
// Red when: ground_speed_at interpolates between steps.
#[test]
fn ground_speed_at_is_the_speed_time_between_flew() {
    let vp = ground(300.25).fly();
    for (a, b, want) in [
        (250.2, 250.1, 300.0),
        (200.2, 199.9, 480.0),
        (45.3, 45.1, 240.0),
        (0.3, 0.1, 240.0),
    ] {
        let flown = 3_600.0 * (a - b) / vp.time_between(a, b);
        let read = vp.ground_speed_at((a + b) / 2.0);
        assert!((flown - read).abs() < 1e-6, "{a}: {flown} {read}");
        assert_eq!(read, want);
    }
}

// AC 4. The ground departure: altitude 200 at 299.75 (0.5 nm into a 1.25 nm, 500 ft step), 20,100 at 250; 0 at
// the start; 83.3 at 0.25 (the last step). Red when: alt_at reads the step's start.
#[test]
fn alt_at_is_linear_between_the_steps_it_was_flown_on() {
    let vp = ground(300.25).fly();
    alts(
        &vp,
        &[
            (299.75, 200.0),
            (250.0, 20_100.0),
            (300.25, 0.0),
            (0.25, 83.3),
        ],
    );
}

// AC 4. Ground over 0 nm (predict's local flight): total 0, distance_after(0, 60) = 0, alt_at and
// ground_speed_at finite, no panic. FLAT from the ground over 0.5 nm: one partial climb step, 0.5 / 300 h =
// 6.0 s. FLAT airborne over 0.5 nm at 100 ft, gs 240, departure 300: 0.5 / 240 h = 7.5 s.
// Red when: a zero-length route loops or divides by zero.
#[test]
fn a_zero_length_route_and_one_shorter_than_a_step_are_timed() {
    let none = ground(0.0).fly();
    assert_eq!(none.time_between(0.0, 0.0), 0.0);
    assert_eq!(none.distance_after(0.0, 60.0), 0.0);
    assert!(none.alt_at(0.0).is_finite() && none.ground_speed_at(0.0).is_finite());
    let short = ground(0.5).fly();
    assert!((short.time_between(0.5, 0.0) - 6.0).abs() < 1e-6);
    let landing = air(100.0, 0.5, 240.0, Some(300.0)).fly();
    assert!((landing.time_between(0.5, 0.0) - 7.5).abs() < 1e-6);
}

// AC 4 (groundspeed limits unchanged). A 100 kt record (envelope 100 to 100) at 30,000 ft into 60 kt: 40 ->
// the 60 kt floor. Observed 51 with a 60 kt headwind along its heading -> scale 0.85: gs 60, not 51.
// FLAT into 400 kt on the nose: 0.4 x 480 = 192; 400 on the tail: 1.6 x 480 = 768.
// Red when: the floor is applied before the scale, or the 0.4 to 1.6 band is dropped.
#[test]
fn the_groundspeed_limits_hold_after_the_scale() {
    let slow = record(300.0, (2_000.0, 100.0), (100.0, 100.0));
    let mut f = air(30_000.0, 200.5, 480.0, Some(300.0));
    f.rec = slow;
    f.gs = None;
    f.wind = RouteWind::uniform(60.0);
    assert_eq!(f.fly().ground_speed_at(200.0), 60.0);
    f.gs = Some((51.0, Some(60.0)));
    assert_eq!(f.fly().ground_speed_at(200.0), 60.0);
    for (hw, want) in [(400.0, 192.0), (-400.0, 768.0)] {
        let mut f = air(30_000.0, 200.5, 480.0, Some(300.0));
        f.gs = None;
        f.wind = RouteWind::uniform(hw);
        assert!((f.fly().ground_speed_at(200.0) - want).abs() < 1e-9, "{hw}");
    }
}

// AC 4. FLAT at cruise in a uniform 40 kt headwind: Some(40) over the whole route. In 80 kt beyond 150 nm and
// 0 inside it, over 200 to 100 at cruise: 40 +/- 1 (one 2 nm step of edge). Still air: None.
// Red when: it is weighted by time (the slower windward half reads above 43).
#[test]
fn mean_headwind_is_weighted_by_distance() {
    let mut f = air(30_000.0, 200.5, 440.0, Some(300.0));
    f.gs = None;
    f.wind = RouteWind::uniform(40.0);
    let mean = f.fly().mean_headwind(200.5, 0.0).unwrap();
    assert!((mean - 40.0).abs() < 1e-9, "{mean}");
    f.wind = RouteWind::new(
        [(0.0, 0.0), (149.9, 0.0), (150.1, 80.0), (201.0, 80.0)]
            .into_iter()
            .map(|(d_nm, hw)| WindKnot {
                d_nm,
                levels: vec![(0.0, hw, None)],
            })
            .collect(),
    );
    let mean = f.fly().mean_headwind(200.0, 100.0).unwrap();
    assert!((mean - 40.0).abs() <= 1.5, "{mean}");
    f.wind = RouteWind::still();
    assert_eq!(f.fly().mean_headwind(200.0, 100.0), None);
}

// AC 4. descent_ground_speed_at: FLAT, sea level, 30,000 / 480: 240 at 40 nm, 480 at 95. Default row FL350 /
// 440: 290 KIAS at the path's 13,333.3 ft = 355.8 kt at 40 nm (next: 363.4), 440 at 150; into 30 kt 325.8
// (the path does not move); 80 kt from FL300 up and calm below 20,000 ft: 355.8 at 40, 360 at 150. Into a
// 5,000 ft field, 40 nm is 18,333.3 ft: 386.3. No anchor and no AC 9 ceiling (open question).
// Red when: it still integrates the rate descent (363.4).
#[test]
fn descent_ground_speed_at_reads_the_path_and_descent_speeds() {
    let rec = record(300.0, (2_000.0, 240.0), (400.0, 500.0));
    let filed = |cruise_ft: f64, tas_kt: f64, arr_elev_ft: f64| Filed {
        cruise_ft,
        tas_kt,
        arr_elev_ft,
    };
    let still = RouteWind::still();
    let at = |d| {
        descent_ground_speed_at(
            d,
            &filed(30_000.0, 480.0, 0.0),
            Performance::Eram(&rec),
            &still,
        )
    };
    assert_eq!(at(40.0), 240.0);
    assert_eq!(at(95.0), 480.0);

    let default = AircraftProfile::default();
    let row = |d, elev, w: &RouteWind| {
        descent_ground_speed_at(
            d,
            &filed(35_000.0, 440.0, elev),
            Performance::Row(&default),
            w,
        )
    };
    let near = |got: f64, want: f64| assert!((got - want).abs() < 0.5, "{got} {want}");
    near(row(40.0, 0.0, &still), 355.8);
    assert_eq!(row(150.0, 0.0, &still), 440.0);
    near(row(40.0, 0.0, &RouteWind::uniform(30.0)), 325.8);
    near(row(40.0, 5_000.0, &still), 386.3);
    let aloft = RouteWind::new(vec![WindKnot {
        d_nm: 0.0,
        levels: vec![
            (0.0, 0.0, None),
            (20_000.0, 0.0, None),
            (30_000.0, 80.0, None),
        ],
    }]);
    near(row(40.0, 0.0, &aloft), 355.8);
    assert_eq!(row(150.0, 0.0, &aloft), 360.0);
}

// AC 5. FLAT filed 43,000 ft, ground, 400 nm: reaches at least 42,500. The C172 row (service_ceiling 14,000)
// filed FL350: never above 14,000, at least 13,500. Red when: the ERAM file's maxAltFt caps the climb, or the
// row's ceiling is lost.
#[test]
fn an_eram_type_climbs_past_where_a_row_is_capped() {
    let mut f = ground(400.0);
    f.cruise = 43_000.0;
    assert!(peak(&f.fly(), 400.0) >= 42_500.0);
    let capped = AircraftProfile {
        service_ceiling_ft: 14_000.0,
        cruise_tas: Some(110.0),
        climb_ias_lo: 75.0,
        climb_ias_hi: 90.0,
        climb_fpm_lo: 500.0,
        climb_fpm_hi: 500.0,
        desc_ias_hi: 110.0,
        desc_ias_lo: 90.0,
        desc_fpm: 500.0,
        ..AircraftProfile::default()
    };
    let top = peak(
        &build_row(&capped, 0.0, 300.0, 35_000.0, 110.0, RouteWind::still()),
        300.0,
    );
    assert!((13_500.0..=14_000.0).contains(&top), "{top}");
}

// AC 7. FLAT, air at 1.21 x ISA at 0 and 30,000 ft (348.66 K and 276.74 K; ISA is linear between, so the
// factor is 1.1 throughout). Airborne 200.5 nm: cruise 480 unchanged, descent 264, total 840 + 88.5 / 264 h =
// 2,046.8 s. Ground 300.25 nm: climb 330 kt at an unchanged 2,000 fpm: altitude 18,272.7 at 250, total
// 3,075.3 s. At 0.81 x ISA: descent 216, total 840 + 88.5 / 216 h = 2,315.0 s. No temperature: 2,167.5 s.
// Red when: the factor is dropped (2,167.5), unrooted (290.4 kt), or reaches cruise (528).
#[test]
fn warm_and_cold_air_scale_the_climb_and_descent_not_the_cruise() {
    let at = |ratio: f64| {
        let mut f = air(30_000.0, 200.5, 480.0, Some(300.0));
        f.wind = air_at(ratio);
        f.fly()
    };
    let warm = at(1.21);
    assert!((warm.ground_speed_at(150.0) - 480.0).abs() < 1e-6);
    assert!(
        (warm.ground_speed_at(40.0) - 264.0).abs() < 0.01,
        "{}",
        warm.ground_speed_at(40.0)
    );
    secs(&warm, 2_046.8);
    let cold = at(0.81);
    assert!((cold.ground_speed_at(40.0) - 216.0).abs() < 0.01);
    secs(&cold, 2_315.0);
    let mut up = ground(300.25);
    up.wind = air_at(1.21);
    let vp = up.fly();
    secs(&vp, 3_075.3);
    alts(&vp, &[(250.0, 18_272.7)]);
}

// AC 8. FLAT at 30,000 ft, 198.5 nm, gs 540 -> scale 1.125. Cruise 2.25 nm steps: 49 -> 88.25 (735 s); then
// 88.25 / 270 h = 1,176.67. Total 1,911.67 s (unscaled 2,152.5). gs 540 at 198.5, 270 at 40.
// Red when: the scale is not applied to the descent (735 + 1,323.75 = 2,058.75).
#[test]
fn an_established_flight_is_scaled_to_its_observed_groundspeed() {
    let vp = air(30_000.0, 198.5, 540.0, Some(300.0)).fly();
    secs(&vp, 1_911.67);
    assert_eq!(vp.ground_speed_at(198.5), 540.0);
    assert_eq!(vp.ground_speed_at(40.0), 270.0);
}

// AC 8. As above: gs 552 -> 552 (1.15, the edge, exact), 600 -> 552, 408 -> 408 (0.85, exact), 300 -> 408. gs at
// 40: 276 and 204. Red when: either bound is dropped or moved (0.9 would give 432).
#[test]
fn the_anchor_is_held_to_fifteen_percent_either_way() {
    for (gs, cruise, descent) in [
        (552.0, 552.0, 276.0),
        (600.0, 552.0, 276.0),
        (408.0, 408.0, 204.0),
        (300.0, 408.0, 204.0),
    ] {
        let vp = air(30_000.0, 198.5, gs, Some(300.0)).fly();
        assert!((vp.ground_speed_at(198.5) - cruise).abs() < 1e-9, "{gs}");
        assert!((vp.ground_speed_at(40.0) - descent).abs() < 1e-9, "{gs}");
    }
}

// AC 8. FLAT 200.5 nm, gs 540, departure 100 (AC 10 off): 29,500 ft -> gs 540; 29,499 ft -> unscaled, 300 then
// 480, total 2,178.75 s; 31,000 ft -> 540 (above counts). Red when: the gate is |altitude - level| <= 500
// (31,000 unscaled), or is 300 ft (29,500 unscaled).
#[test]
fn only_a_flight_within_500_ft_of_its_level_or_above_it_is_scaled() {
    let at = |alt: f64| air(alt, 200.5, 540.0, Some(100.0)).fly();
    assert_eq!(at(29_500.0).ground_speed_at(200.5), 540.0);
    assert_eq!(at(31_000.0).ground_speed_at(200.5), 540.0);
    let low = at(29_499.0);
    assert_eq!(low.ground_speed_at(200.5), 300.0);
    secs(&low, 2_178.75);
}

// AC 8, in place of the 250 kt gate. A record cruising 200 to 260, filed 230 at 20,000 ft, 150.5 nm, gs 240:
// scale 240 / 230, gs 240 at the start. FLAT at 30,000 ft, gs 240: clamped, 408, not the unscaled 480.
// Red when: the 250 kt minimum is kept.
#[test]
fn a_slow_flight_at_cruise_is_scaled() {
    let mut f = air(20_000.0, 150.5, 240.0, Some(300.0));
    f.rec = record(150.0, (1_000.0, 150.0), (200.0, 260.0));
    f.cruise = 20_000.0;
    f.tas = 230.0;
    assert!((f.fly().ground_speed_at(150.5) - 240.0).abs() < 1e-9);
    let jet = air(30_000.0, 198.5, 240.0, Some(300.0)).fly();
    assert!((jet.ground_speed_at(198.5) - 408.0).abs() < 1e-9);
}

// AC 8, AC 9. A ground flight's groundspeed is inert: FLAT ground over 300.25 nm with gs 0 and 45 gives
// 3,247.5 s, and over 60 nm 817.5 s. A ground start at 6,000 ft filed at 5,000 is not scaled either.
// Red when: the airborne test is dropped from either rule.
#[test]
fn a_ground_flight_is_never_scaled_or_capped() {
    secs(&ground(300.25).fly(), 3_247.5);
    secs(&ground(60.0).fly(), 817.5);
    // On the ground above its filed altitude: still no scale.
    let mut f = ground(100.5);
    f.alt = 6_000.0;
    f.cruise = 5_000.0;
    assert_eq!(f.fly().ground_speed_at(100.0), 480.0);
}

// AC 8. FLAT 30,000 ft, 198.5 nm, gs 432, route wind a uniform 48 kt headwind. Headwind along its heading 48
// -> predicted 432, scale 1, gs 432 at the start. Heading headwind 0 (heading across the wind) -> predicted
// 480, scale 0.9, gs (480 - 48) x 0.9 = 388.8. Red when: the route's headwind is used for both.
#[test]
fn the_anchor_takes_the_headwind_it_is_given_not_the_routes() {
    let at = |heading_hw: f64| {
        let mut f = air(30_000.0, 198.5, 432.0, Some(300.0));
        f.wind = RouteWind::uniform(48.0);
        f.gs = Some((432.0, Some(heading_hw)));
        f.fly().ground_speed_at(198.5)
    };
    assert!((at(48.0) - 432.0).abs() < 1e-9, "{}", at(48.0));
    assert!((at(0.0) - 388.8).abs() < 1e-9, "{}", at(0.0));
}

// AC 8 with AC 10. FLAT, 20,000 ft, 200.5 nm, departure 113: gs 420 -> 0.875, gs 420 at 199 and 95, 210 at 61;
// 199 to 99 takes 100 / 420 h = 857.1 s. gs 300 -> clamped, 408. Open question: with an envelope that differs
// between 20,000 and 30,000 ft, AC 8's "at that level" reads the 20,000 ft value and AC 6's "cruise altitude"
// the 30,000. Red when: the gate compares against the filed altitude (unscaled, 480).
#[test]
fn a_stepped_down_flight_is_scaled_at_its_present_level() {
    let vp = air(20_000.0, 200.5, 420.0, Some(113.0)).fly();
    for (d, gs) in [(199.0, 420.0), (95.0, 420.0), (61.0, 210.0)] {
        assert!((vp.ground_speed_at(d) - gs).abs() < 1e-9, "{d}");
    }
    assert!((vp.time_between(199.0, 99.0) - 857.14).abs() < 0.1);
    let slow = air(20_000.0, 200.5, 300.0, Some(113.0)).fly();
    assert!((slow.ground_speed_at(199.0) - 408.0).abs() < 1e-9);
    // Its cruise speed range is read at its level: 300 to 400 kt at 20,000 ft, 400 to 500 at 30,000.
    let mut f = air(20_000.0, 200.5, 380.0, Some(113.0));
    f.rec = sloped_envelope();
    assert!((f.fly().ground_speed_at(199.0) - 380.0).abs() < 1e-9);
    f.gs = Some((600.0, None));
    assert!(
        (f.fly().ground_speed_at(199.0) - 460.0).abs() < 1e-9,
        "1.15 x 400"
    );
}

// AC 9. FLAT at 0 ft (CAS = TAS), 30 nm out, departure 200, predicted 240. gs 225: exactly 15 slower, not
// capped, 450 s. gs 224: capped, 30 / 224 h = 482.14 s, gs 224 at 29.5 and 0.5. gs 220 capped, 230 not.
// Red when: the deadband is >=, 10 or 20.
#[test]
fn a_descent_slower_by_more_than_15_kt_cas_is_capped_at_its_observed_speed() {
    let at = |gs: f64| air(0.0, 30.0, gs, Some(200.0)).fly();
    secs(&at(225.0), 450.0);
    secs(&at(230.0), 450.0);
    let capped = at(224.0);
    secs(&capped, 482.14);
    for d in [29.5, 0.5] {
        assert!((capped.ground_speed_at(d) - 224.0).abs() < 1e-9);
    }
    secs(&at(220.0), 490.91);
}

// AC 9. FLAT, 20,000 ft, 60 nm, departure 200, gs 205.5 (150 KCAS at 20,000 ft): gs 205.5 at 59.5, 175.2 at
// 30.25 and 150.7 at 0.5, +/- 0.5 (150 KCAS at each step's starting altitude). Total 1,237.8 s +/- 1
// (uncapped 900). Red when: the TAS is held (205.5 throughout) or the ceiling ends early.
#[test]
fn the_assigned_speed_is_a_cas_held_to_the_field() {
    let vp = air(20_000.0, 60.0, 205.5, Some(200.0)).fly();
    for (d, gs) in [(59.5, 205.5), (30.25, 175.2), (0.5, 150.7)] {
        let got = vp.ground_speed_at(d);
        assert!((got - gs).abs() <= 0.5, "{d}: {got}");
    }
    let t = vp.time_between(60.0, 0.0);
    assert!((t - 1_237.8).abs() <= 1.0, "{t}");
}

// AC 9. LONG at 30,000 ft, 120.5 nm (inside 130, and at its level so AC 8 applies), departure 300, gs 150:
// anchor 150 / 480 -> 0.85, predicted 240 x 0.85 = 204 = 124.8 KCAS; observed 91.7 KCAS, 33 slower: capped,
// scale dropped. gs 150.0 at 120, not 127.5; 96.5 +/- 0.5 at 10. Red when: the scale survives.
#[test]
fn a_capped_flight_is_not_also_scaled() {
    let mut f = long();
    f.d = 120.5;
    f.gs = Some((150.0, None));
    let vp = f.fly();
    assert!(
        (vp.ground_speed_at(120.0) - 150.0).abs() < 1e-6,
        "{}",
        vp.ground_speed_at(120.0)
    );
    assert!(
        (vp.ground_speed_at(10.0) - 96.5).abs() <= 0.5,
        "{}",
        vp.ground_speed_at(10.0)
    );
}

// AC 9. The same flight at gs 360: anchor 0.75 -> 0.85, predicted 204; 360 is faster, so no ceiling and the
// scale stays: gs 204 at 120 and 10, total 120.5 / 204 h = 2,126.5 s. At gs 200 (122.3 KCAS, 2.5 slower,
// inside the deadband): 204. Red when: observed is compared with the unscaled 240 (200 would cap).
#[test]
fn a_flight_matching_its_scaled_descent_keeps_the_scale() {
    for gs in [360.0, 200.0] {
        let mut f = long();
        f.d = 120.5;
        f.gs = Some((gs, None));
        let vp = f.fly();
        for d in [120.0, 10.0] {
            assert!((vp.ground_speed_at(d) - 204.0).abs() < 1e-9, "{gs} at {d}");
        }
        secs(&vp, 2_126.47);
    }
}

// AC 9. FLAT at 0 ft, 30 nm: gs 300 and gs 240 both 450 s. At 10,000 ft, 30 nm, departure 200: gs 400 and
// gs 240 both 450 s. Red when: the observed speed replaces the prediction in both directions.
#[test]
fn a_faster_descent_changes_nothing() {
    for (alt, gs) in [
        (0.0, 300.0),
        (0.0, 240.0),
        (10_000.0, 400.0),
        (10_000.0, 240.0),
    ] {
        secs(&air(alt, 30.0, gs, Some(200.0)).fly(), 450.0);
    }
}

// AC 9. FLAT at 0 ft, 30 nm, still route wind. Headwind along the heading +50, gs 224 -> TAS 274: not capped,
// 450 s. Tailwind (-30), gs 250 -> TAS 220: capped at 220, 490.9 s. Red when: the wind is ignored or its
// sign is flipped.
#[test]
fn the_observed_cas_adds_the_headwind_along_the_heading() {
    let at = |gs: f64, hw: f64| {
        let mut f = air(0.0, 30.0, gs, Some(200.0));
        f.gs = Some((gs, Some(hw)));
        f.fly()
    };
    secs(&at(224.0, 50.0), 450.0);
    secs(&at(250.0, -30.0), 490.91);
}

// AC 9. The 60 nm airborne flight at 5,000 ft, departure 40 (climbing under AC 2): gs 150 and gs 300 both
// 836.25 s. Control: departure 80 with gs 150 is capped (139.2 KCAS), 1,453.3 s +/- 1 against 900.
#[test]
fn a_short_trip_still_climbing_is_not_capped() {
    for gs in [150.0, 300.0] {
        secs(&air(5_000.0, 60.0, gs, Some(40.0)).fly(), 836.25);
    }
    let t = air(5_000.0, 60.0, 150.0, Some(80.0))
        .fly()
        .time_between(60.0, 0.0);
    assert!((t - 1_453.3).abs() <= 1.0, "{t}");
    // Nearer its departure but already on its level: not climbing, so an assigned speed holds it.
    let mut level = air(8_000.0, 60.0, 150.0, Some(40.0));
    level.cruise = 8_000.0;
    level.rec = record(300.0, (300.0, 240.0), (400.0, 500.0));
    assert!(level.fly().ground_speed_at(59.5) < 160.0);
}

// AC 9. FLAT, 10,000 ft, 200.5 nm, departure 40 (climbing, outside 90): gs 200 and gs 300 both 2,396.25 s.
// Control: at 80 nm, departure 300, gs 200 is capped at 171.9 KCAS. Red when: the top-of-descent test is dropped.
#[test]
fn a_slow_climb_outside_top_of_descent_is_not_capped() {
    for gs in [200.0, 300.0] {
        secs(&air(10_000.0, 200.5, gs, Some(40.0)).fly(), 2_396.25);
    }
    let vp = air(10_000.0, 80.0, 200.0, Some(300.0)).fly();
    assert!((vp.ground_speed_at(79.5) - 200.0).abs() < 0.5);
}

// AC 10. FLAT, 20,000 ft, 200.5 nm, gs 420, 1.5 x 75 = 112.5. Departure 113: level, altitude 20,000 at 150 and
// 61, total 2,477.1 s (1.75 nm steps at 420 kt: 64 -> 88.5 nm, 960 s; 88.5 / 210 h = 1,517.1 s). Departure
// 112.5 (not farther) and 111: climbs, altitude 29,500 at 150, total 2,276.25 s.
// Red when: the margin is 1.0 (111 level) or 2.0 (113 climbs), or the test is >= (112.5 level).
#[test]
fn a_flight_past_one_and_a_half_climb_distances_is_level_where_it_is() {
    let level = air(20_000.0, 200.5, 420.0, Some(113.0)).fly();
    alts(&level, &[(150.0, 20_000.0), (61.0, 20_000.0)]);
    secs(&level, 2_477.14);
    for dep in [112.5, 111.0] {
        let vp = air(20_000.0, 200.5, 420.0, Some(dep)).fly();
        alts(&vp, &[(150.0, 29_500.0)]);
        secs(&vp, 2_276.25);
    }
    // Within 500 ft of its cruise altitude it is at cruise, whatever its distance.
    let near = air(29_500.0, 200.5, 480.0, Some(400.0)).fly();
    alts(&near, &[(150.0, 29_500.0)]);
}

// AC 10. The same flight with no known departure: climbs, total 2,276.25 s, and its groundspeed is not read
// (gs 300 gives the same total). Red when: unknown is read as far.
#[test]
fn a_low_flight_with_no_known_departure_climbs() {
    for gs in [420.0, 300.0] {
        secs(&air(20_000.0, 200.5, gs, None).fly(), 2_276.25);
    }
}

// AC 10. FLAT, 20,000 ft, 80 nm (inside 90), departure 300, gs 270: altitude 20,000 to the path at 60 (AC 2),
// but not a level flight: gs 240 at 79.5, not scaled to 270. Total 1,200 s. Red when: AC 10 is judged before
// top of descent and the anchor fires.
#[test]
fn a_low_flight_inside_top_of_descent_is_not_made_level() {
    let vp = air(20_000.0, 80.0, 270.0, Some(300.0)).fly();
    assert_eq!(vp.ground_speed_at(79.5), 240.0);
    alts(&vp, &[(61.0, 20_000.0)]);
    secs(&vp, 1_200.0);
}

// AC 10. The level flight at 20,000 ft (departure 113, gs 480, so unscaled): cruise speed 480 at 91, descent
// speed 240 at 85 though its own altitude meets the path only at 60 nm; altitude 20,000 at 61, 19,666.7 at 59;
// total 2,167.5 s. Red when: descent_start is recomputed from the level (60, or 40 + 10).
#[test]
fn a_level_flights_top_of_descent_is_still_its_filed_altitudes() {
    let vp = air(20_000.0, 200.5, 480.0, Some(113.0)).fly();
    assert_eq!(vp.ground_speed_at(91.0), 480.0);
    assert_eq!(vp.ground_speed_at(85.0), 240.0);
    alts(&vp, &[(61.0, 20_000.0), (59.0, 19_666.7)]);
    secs(&vp, 2_167.5);
}
