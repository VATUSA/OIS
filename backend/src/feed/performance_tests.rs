//! #985: the ERAM table, how a record is read, and how a flight resolves to one.
//!
//! Coverage matrix (VATUSA/OIS#985)
//!
//! Requirement                                     | Test
//! ------------------------------------------------|-----------------------------------------------------------------------
//! AC 5 the committed file, whole                  | the_bundled_file_holds_2692_types_each_with_all_four_tables           | OK
//! AC 5 its rows ascend and can be flown           | every_bundled_table_ascends_in_altitude_with_usable_rates             | OK
//! AC 5 B738 as committed                          | the_bundled_b738_reads_its_committed_rows                             | OK
//! AC 5 C172 as committed                          | the_bundled_c172_reads_its_committed_rows                             | OK
//! AC 5 a bad file fails at parse                  | a_file_missing_a_table_or_the_type_map_is_refused                     | OK
//! AC 5 climb and descent tables                   | climb_and_descent_interpolate_between_rows_and_hold_past_the_ends     | OK
//! AC 5 type row > ERAM > family > wake > default  | a_type_resolves_type_row_then_eram_then_family_then_wake_then_default | OK
//! AC 5 label agrees at every tier                 | the_label_names_the_source_resolve_returned                           | OK
//! AC 5 case                                       | an_eram_type_is_matched_whatever_its_case                             | OK
//! AC 5 a default table has no ERAM                | the_default_table_holds_no_eram_record                                | OK
//! AC 5 no ceiling from ERAM                       | an_eram_record_has_no_ceiling_and_a_row_keeps_its_own                 | OK
//! AC 6 filed speed held in the envelope           | an_eram_cruise_is_the_filed_speed_held_inside_the_envelope            | OK
//! AC 6 60 and 700 kt bounds                       | a_filed_speed_outside_60_to_700_kt_cruises_at_the_midpoint            | OK
//! AC 6 envelope read at the cruise altitude       | the_envelope_is_read_at_the_cruise_altitude                           | OK
//! AC 6 rows cruise as today                       | a_rows_cruise_speed_is_unchanged                                      | OK
//! AC 7 scaling at a non-identity temperature      | eram_climb_and_descent_scale_with_the_root_of_the_temperature_ratio   | OK
//! AC 7 not cruise, not the rate, not rows         | temperature_scales_neither_cruise_nor_a_rate_nor_a_row                | OK
//! AC 7 no temperature                             | without_a_temperature_the_tables_are_read_as_written                  | OK
//! AC 3 the type's own descent distance            | the_descent_distance_integrates_the_descent_table_to_the_field        | OK
//! AC 10 the type's climb distance                 | the_climb_distance_integrates_the_climb_table_from_the_surface        | OK
//! AC 3, 10 a row's distances                      | a_rows_distances_integrate_its_schedule                               | OK
//! AC 3 never faster in descent than cruise        | a_descent_speed_is_held_to_the_cruise_speed                           | OK
//! AC 9 CAS and TAS                                | tas_to_cas_inverts_ias_to_tas                                         | OK
//!
//! REC, the record the pure cases use (a jet shape, so every segment slopes):
//!   climb   [0,3000,250] [10000,2500,300] [20000,2000,400] [30000,1000,450]
//!   descent [0,1800,250] [10000,1700,290] [20000,2000,400] [30000,1300,440]
//!   min     [3000,210] [30000,420]        max [3000,260] [30000,490]
//! Each climb and descent row is `[altitude ft, fpm, TAS kt]`, each envelope row `[altitude ft, TAS]`.

use std::collections::HashMap;

use super::super::trajectory::{AircraftProfile, ProfileTable, ias_to_tas, isa_temp_k, tas_to_cas};
use super::{EramRecord, EramTable, Performance};

/// ISA at 10,000 ft (K): 288.15 - 19.812.
const ISA_10K: f64 = 268.338;

fn close(a: f64, b: f64, within: f64) -> bool {
    (a - b).abs() <= within
}

fn eram(ty: &str) -> Performance<'static> {
    Performance::Eram(EramTable::bundled().get(ty).expect("a bundled type"))
}

/// REC.
fn rec() -> EramRecord {
    EramRecord {
        climb: vec![
            [0.0, 3_000.0, 250.0],
            [10_000.0, 2_500.0, 300.0],
            [20_000.0, 2_000.0, 400.0],
            [30_000.0, 1_000.0, 450.0],
        ],
        descent: vec![
            [0.0, 1_800.0, 250.0],
            [10_000.0, 1_700.0, 290.0],
            [20_000.0, 2_000.0, 400.0],
            [30_000.0, 1_300.0, 440.0],
        ],
        min_tas: vec![[3_000.0, 210.0], [30_000.0, 420.0]],
        max_tas: vec![[3_000.0, 260.0], [30_000.0, 490.0]],
    }
}

/// The same 2,000 fpm climb and descent at every altitude, cruising 400 to 500 kt.
fn flat(climb_kt: f64, descent_kt: f64) -> EramRecord {
    EramRecord {
        climb: vec![[0.0, 2_000.0, climb_kt], [60_000.0, 2_000.0, climb_kt]],
        descent: vec![[0.0, 2_000.0, descent_kt], [60_000.0, 2_000.0, descent_kt]],
        min_tas: vec![[0.0, 400.0]],
        max_tas: vec![[0.0, 500.0]],
    }
}

fn row(desc_fpm: f64) -> AircraftProfile {
    AircraftProfile {
        desc_fpm,
        ..AircraftProfile::default()
    }
}

/// Every tier at its own `desc_fpm`, over the bundled ERAM table.
fn tiers(with_type_row: bool) -> ProfileTable {
    ProfileTable::new(
        row(4_444.0),
        HashMap::from([("H".to_string(), row(3_333.0))]),
        HashMap::from([("jet".to_string(), row(2_222.0))]),
        if with_type_row {
            HashMap::from([("B772".to_string(), row(1_111.0))])
        } else {
            HashMap::new()
        },
        Some(EramTable::bundled()),
    )
}

// AC 5. Input: `EramTable::bundled()`. Expected: exactly 2_692 types, every one with non-empty climb,
// descent, min and max tables; name the offending type in the assertion.
// Red when: a record or a table is dropped from backend/data/aircraft-performance.json.
#[test]
fn the_bundled_file_holds_2692_types_each_with_all_four_tables() {
    let types = &EramTable::bundled().0;
    assert_eq!(types.len(), 2_692);
    for (ty, r) in types {
        assert!(
            !r.climb.is_empty()
                && !r.descent.is_empty()
                && !r.min_tas.is_empty()
                && !r.max_tas.is_empty(),
            "{ty}"
        );
    }
}

// AC 5. Input: every record of `EramTable::bundled()`. Expected: altitudes strictly increase in all four
// tables (the interpolation brackets on them), every climb and descent rate >= 100 fpm, every TAS >= 70 kt
// (the file's own minima: climb 100 fpm at A9 25,000 ft, descent 300 fpm, TAS 70).
// Red when: a zero rate (an endless climb) or an unsorted row is committed.
#[test]
fn every_bundled_table_ascends_in_altitude_with_usable_rates() {
    fn ascends<const N: usize>(rows: &[[f64; N]]) -> bool {
        rows.windows(2).all(|w| w[0][0] < w[1][0])
    }
    for (ty, r) in &EramTable::bundled().0 {
        assert!(
            ascends(&r.climb) && ascends(&r.descent) && ascends(&r.min_tas) && ascends(&r.max_tas),
            "{ty}"
        );
        for row in r.climb.iter().chain(&r.descent) {
            assert!(row[1] >= 100.0 && row[2] >= 70.0, "{ty} {row:?}");
        }
        for (lo, hi) in r.min_tas.iter().zip(&r.max_tas) {
            assert!(lo[1] >= 70.0 && hi[1] >= lo[1], "{ty} {lo:?} {hi:?}");
        }
    }
}

// AC 5. B738 at ISA, from the committed file (vTBFM's performance.test.ts pins the same numbers):
//   climb 10,000 ft: 289 kt, 3,067 fpm (a row).
//   climb 30,000 ft: between [28000,1570,440] and [30555,1600,458], f = 2000/2555 = 0.78278:
//     440 + 0.78278 x 18 = 454.09 kt; 1570 + 0.78278 x 30 = 1593.48 fpm.
//   descent 15,000 ft: 379 kt. Cruise at FL350 filed 450: 450; unfiled: min at 35,000 ft =
//   430 + 2/3 x 10 = 436.667, max = 487 - 2/3 x 11 = 479.667, midpoint 458.167.
//   Descent distance FL350 to sea level 130.884 nm; climb distance to FL350 105.884 nm (500 ft steps).
// Red when: a row is edited, or the columns are read in the wrong order.
#[test]
fn the_bundled_b738_reads_its_committed_rows() {
    let p = eram("B738");
    assert_eq!(p.climb(10_000.0, None), (289.0, 3_067.0));
    let (tas, fpm) = p.climb(30_000.0, None);
    assert!(
        close(tas, 454.09, 0.01) && close(fpm, 1_593.48, 0.01),
        "{tas} {fpm}"
    );
    assert!(close(p.descent_tas(15_000.0, None, 450.0), 379.0, 1e-9));
    assert_eq!(p.cruise_tas(450.0, 35_000.0, 35_000.0), 450.0);
    let unfiled = p.cruise_tas(0.0, 35_000.0, 35_000.0);
    assert!(close(unfiled, 458.167, 0.001), "{unfiled}");
    let down = p.descent_distance_nm(35_000.0, 0.0, 450.0);
    assert!(close(down, 130.884, 0.001), "{down}");
    let up = p.climb_distance_nm(35_000.0);
    assert!(close(up, 105.884, 0.001), "{up}");
}

// AC 5. C172: climb 6,000 ft is 2/3 of the way from [4000,543,82] to [7000,414,85]: 84 kt, 457 fpm.
// Cruise unfiled at 6,000 ft: (95 + 130) / 2 = 112.5 kt, well above its climb speed.
// Red when: cruise falls back to the climb table's TAS.
#[test]
fn the_bundled_c172_reads_its_committed_rows() {
    let p = eram("C172");
    let (tas, fpm) = p.climb(6_000.0, None);
    assert!(
        close(tas, 84.0, 1e-9) && close(fpm, 457.0, 1e-9),
        "{tas} {fpm}"
    );
    let unfiled = p.cruise_tas(0.0, 6_000.0, 6_000.0);
    assert!(close(unfiled, 112.5, 0.01), "{unfiled}");
}

// AC 5 (design: a bad file fails at boot). Inputs, each through the parser `bundled()` uses: not JSON;
// `{}`; a record without `maxTas`; a record with an empty `climb`. Expected: an error each; the same file
// with the record whole parses (control).
// Red when: a missing table deserializes to an empty one and the type flies at 0 kt.
#[test]
fn a_file_missing_a_table_or_the_type_map_is_refused() {
    let whole = r#"{"types":{"X":{"climb":[[0,1,100]],"descent":[[0,1,100]],"minTas":[[0,100]],"maxTas":[[0,100]]}}}"#;
    assert!(EramTable::parse(whole).is_ok(), "control");
    for bad in [
        "not json".to_string(),
        "{}".to_string(),
        r#"{"types":{}}"#.to_string(),
        whole.replace(r#","maxTas":[[0,100]]"#, ""),
        whole.replace(r#""climb":[[0,1,100]]"#, r#""climb":[]"#),
        whole.replace(r#""descent":[[0,1,100]]"#, r#""descent":[]"#),
        whole.replace(r#""minTas":[[0,100]]"#, r#""minTas":[]"#),
        whole.replace(r#""maxTas":[[0,100]]"#, r#""maxTas":[]"#),
    ] {
        assert!(EramTable::parse(&bad).is_err(), "{bad}");
    }
}

// AC 5. REC at ISA. climb 5,000 ft: 275 kt, 2,750 fpm. climb 45,000 ft (past the last row): 450 kt,
// 1,000 fpm. climb -500 ft: 250 kt, 3,000 fpm. climb 29,000 ft (inside the last segment): 400 + 0.9 x 50
// = 445 kt. descent 15,000 ft: (290 + 400) / 2 = 345 kt.
// Red when: the last segment is skipped, or an end extrapolates.
#[test]
fn climb_and_descent_interpolate_between_rows_and_hold_past_the_ends() {
    let r = rec();
    let p = Performance::Eram(&r);
    assert_eq!(p.climb(5_000.0, None), (275.0, 2_750.0));
    assert_eq!(p.climb(45_000.0, None), (450.0, 1_000.0));
    assert_eq!(p.climb(-500.0, None), (250.0, 3_000.0));
    assert_eq!(p.climb(20_000.0, None), (400.0, 2_000.0));
    assert!(close(p.climb(29_000.0, None).0, 445.0, 1e-9));
    assert!(close(p.descent_tas(15_000.0, None, 500.0), 345.0, 1e-9));
}

// AC 5. Literal `desc_fpm` per row tier (1,111 type B772, 2,222 family jet, 3,333 wake H, 4,444 default)
// and the bundled ERAM table:
//   B772 H, type row present -> Row 1,111  (an operator's row beats ERAM)
//   B772 H, no type row      -> Eram B772  (beats the jet family row)
//   E175 H, no type row      -> Row 2,222  (a jet in ENGINE_REF the file lacks)
//   QQQQ H                   -> Row 3,333  (in neither)
//   QQQQ ""                  -> Row 4,444
// Red when: any two adjacent tiers are swapped (one mutation per pair).
#[test]
fn a_type_resolves_type_row_then_eram_then_family_then_wake_then_default() {
    for (ty, wake, with_type_row, want) in [
        ("B772", "H", true, Some(1_111.0)),
        ("B772", "H", false, None),
        ("E175", "H", false, Some(2_222.0)),
        ("QQQQ", "H", false, Some(3_333.0)),
        ("QQQQ", "", false, Some(4_444.0)),
    ] {
        let t = tiers(with_type_row);
        match (t.resolve(ty, wake), want) {
            (Performance::Row(p), Some(fpm)) => assert_eq!(p.desc_fpm, fpm, "{ty} {wake}"),
            (Performance::Eram(r), None) => {
                assert_eq!(r.climb, EramTable::bundled().get(ty).unwrap().climb, "{ty}")
            }
            (got, _) => panic!("{ty} {wake}: {got:?}"),
        }
    }
}

// AC 5, AC 12. The five cases above: "type:B772", "eram:B772", "family:jet", "wake:H", "default".
// Red when: the label is computed apart from the tier `resolve` matched.
#[test]
fn the_label_names_the_source_resolve_returned() {
    for (ty, wake, with_type_row, want) in [
        ("B772", "H", true, "type:B772"),
        ("B772", "H", false, "eram:B772"),
        ("E175", "H", false, "family:jet"),
        ("QQQQ", "H", false, "wake:H"),
        ("QQQQ", "", false, "default"),
    ] {
        assert_eq!(tiers(with_type_row).resolve_label(ty, wake), want);
    }
}

// AC 5. "b738" resolves to Eram B738, label "eram:B738"; a type row stored as "B738" beats it for "b738".
// Red when: the ERAM lookup skips the uppercasing the type tier does.
#[test]
fn an_eram_type_is_matched_whatever_its_case() {
    let t = tiers(false);
    assert!(matches!(t.resolve("b738", ""), Performance::Eram(_)));
    assert_eq!(t.resolve_label("b738", ""), "eram:B738");
    let own = ProfileTable::new(
        row(4_444.0),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([("B738".to_string(), row(1_111.0))]),
        Some(EramTable::bundled()),
    );
    assert_eq!(own.resolve_label("b738", ""), "type:B738");
}

// AC 5 (design). `ProfileTable::default()` resolves B738 to the default row, label "default": the fixture
// every older test builds on. Control: the same rows with the bundled ERAM give "eram:B738".
#[test]
fn the_default_table_holds_no_eram_record() {
    let t = ProfileTable::default();
    assert_eq!(t.resolve_label("B738", ""), "default");
    assert!(matches!(t.resolve("B738", ""), Performance::Row(_)));
    assert_eq!(
        ProfileTable::built_in().resolve_label("B738", ""),
        "eram:B738"
    );
    assert_eq!(
        ProfileTable::built_in().resolve_label("QQQQ", "H"),
        "default"
    );
}

// AC 5. Eram C172 (maxAltFt 45,000 in the file) has no ceiling; a row with service_ceiling_ft 14,000
// reports 14,000. Red when: the file's maxAltFt is applied.
#[test]
fn an_eram_record_has_no_ceiling_and_a_row_keeps_its_own() {
    assert_eq!(eram("C172").ceiling_ft(), None);
    let own = AircraftProfile {
        service_ceiling_ft: 14_000.0,
        ..AircraftProfile::default()
    };
    assert_eq!(Performance::Row(&own).ceiling_ft(), Some(14_000.0));
}

// AC 6. REC at 30,000 ft (envelope 420 to 490): filed 450 -> 450, 600 -> 490, 300 -> 420, 420 -> 420,
// 490 -> 490. Red when: either side of the clamp is dropped.
#[test]
fn an_eram_cruise_is_the_filed_speed_held_inside_the_envelope() {
    let r = rec();
    let p = Performance::Eram(&r);
    for (filed, want) in [
        (450.0, 450.0),
        (600.0, 490.0),
        (300.0, 420.0),
        (420.0, 420.0),
        (490.0, 490.0),
    ] {
        assert_eq!(p.cruise_tas(filed, 30_000.0, 30_000.0), want, "{filed}");
    }
}

// AC 6. REC at 30,000 ft, midpoint 455: filed 0 -> 455; 59 -> 455; 60 -> 420 (believed, then clamped);
// 700 -> 490; 701 -> 455; 4800 -> 455. 59/60 and 700/701 give different answers, so each bound is pinned
// from both sides. Red when: either bound moves by one or becomes exclusive.
#[test]
fn a_filed_speed_outside_60_to_700_kt_cruises_at_the_midpoint() {
    let r = rec();
    let p = Performance::Eram(&r);
    for (filed, want) in [
        (0.0, 455.0),
        (59.0, 455.0),
        (60.0, 420.0),
        (700.0, 490.0),
        (701.0, 455.0),
        (4_800.0, 455.0),
    ] {
        assert_eq!(p.cruise_tas(filed, 30_000.0, 30_000.0), want, "{filed}");
    }
}

// AC 6. REC at a 16,500 ft cruise: f = 0.5 -> min 315, max 375; filed 450 -> 375, unfiled 345. Below the
// first row, 2,000 ft: 210 to 260, unfiled 235. Red when: the envelope is read at a fixed altitude.
#[test]
fn the_envelope_is_read_at_the_cruise_altitude() {
    let r = rec();
    let p = Performance::Eram(&r);
    // The level, not the filing: a flight held below its filed altitude cruises at its level.
    for level in [(16_500.0, 375.0, 345.0), (2_000.0, 260.0, 235.0)] {
        assert_eq!(p.cruise_tas(450.0, 30_000.0, level.0), level.1);
        assert_eq!(p.cruise_tas(0.0, 30_000.0, level.0), level.2);
    }
}

// AC 6. Rows: default row filed 440 -> 440; unfiled at FL350 -> 440, FL160 -> 300, 5,000 ft ->
// 170; cruise_tas 110 caps 400 to 110, leaves 100 alone and replaces 0; cruise_mach 0.84 at 218.808 K caps 520 to
// 661.4788 x 0.84 x sqrt(218.808 / 288.15) = 484.2, and at 233.808 K to 500.5.
// Red when: the ERAM envelope rule reaches rows, or the Mach cap loses the air temperature.
#[test]
fn a_rows_cruise_speed_is_unchanged() {
    let default = AircraftProfile::default();
    let p = Performance::Row(&default);
    for (filed, alt, want) in [
        (440.0, 35_000.0, 440.0),
        (0.0, 35_000.0, 440.0),
        (0.0, 16_000.0, 300.0),
        (0.0, 5_000.0, 170.0),
    ] {
        // The level a row cruises at changes nothing: only the filing is read.
        assert_eq!(p.cruise_tas(filed, alt, 20_000.0), want, "{filed} {alt}");
    }
    let slow = AircraftProfile {
        cruise_tas: Some(110.0),
        ..AircraftProfile::default()
    };
    assert_eq!(
        Performance::Row(&slow).cruise_tas(400.0, 8_000.0, 8_000.0),
        110.0
    );
    assert_eq!(
        Performance::Row(&slow).cruise_tas(100.0, 8_000.0, 8_000.0),
        100.0
    );
    assert_eq!(
        Performance::Row(&slow).cruise_tas(0.0, 8_000.0, 8_000.0),
        110.0
    );
    let mach = AircraftProfile {
        cruise_mach: Some(0.84),
        ..AircraftProfile::default()
    };
    let held = |temp| Performance::Row(&mach).cruise_in(520.0, 35_000.0, temp);
    assert!(close(held(None), 484.2, 0.1), "{}", held(None));
    assert!(
        close(held(Some(233.808)), 500.5, 0.1),
        "{}",
        held(Some(233.808))
    );
    assert_eq!(p.cruise_in(520.0, 35_000.0, Some(233.808)), 520.0);
}

// AC 7. REC at 10,000 ft (ISA 288.15 - 19.812 = 268.338 K). At 1.21 x 268.338 = 324.689 K the factor is
// sqrt(1.21) = 1.1 -> climb 330 kt (300), descent 319 kt (290). At 0.81 x ISA = 217.354 K -> 270 and 261.
// Red when: the ratio is not rooted (1.21 -> 363), or ISA is read at sea level.
#[test]
fn eram_climb_and_descent_scale_with_the_root_of_the_temperature_ratio() {
    let r = rec();
    let p = Performance::Eram(&r);
    for (ratio, climb, descent) in [(1.21, 330.0, 319.0), (0.81, 270.0, 261.0)] {
        let temp = Some(ISA_10K * ratio);
        assert!(close(p.climb(10_000.0, temp).0, climb, 1e-6), "{ratio}");
        assert!(
            close(p.descent_tas(10_000.0, temp, 500.0), descent, 1e-6),
            "{ratio}"
        );
    }
}

// AC 7. Same air: REC climb rate at 10,000 ft stays 2,500 fpm; cruise filed 450 at 30,000 ft stays 450 at
// 1.21 x ISA; a default row's climb and descent TAS at 10,000 ft are the ISA values.
// Control: the same call's ERAM climb TAS did move. Red when: the factor is applied to any of the three.
#[test]
fn temperature_scales_neither_cruise_nor_a_rate_nor_a_row() {
    let r = rec();
    let p = Performance::Eram(&r);
    let hot = Some(ISA_10K * 1.21);
    assert_eq!(p.climb(10_000.0, hot).1, 2_500.0);
    assert!(p.climb(10_000.0, hot).0 > 320.0, "control: the speed moved");
    assert_eq!(p.cruise_in(450.0, 30_000.0, Some(276.74)), 450.0);
    let default = AircraftProfile::default();
    let own = Performance::Row(&default);
    assert_eq!(own.climb(10_000.0, hot), own.climb(10_000.0, None));
    assert_eq!(
        own.descent_tas(10_000.0, hot, 440.0),
        own.descent_tas(10_000.0, None, 440.0)
    );
}

// AC 7. No temperature -> factor 1: REC climb 10,000 ft 300 kt, descent 290 kt, exactly the ISA answer.
// Red when: a missing temperature is read as 0 K or as sea-level ISA.
#[test]
fn without_a_temperature_the_tables_are_read_as_written() {
    let r = rec();
    let p = Performance::Eram(&r);
    assert_eq!(p.climb(10_000.0, None).0, 300.0);
    assert_eq!(p.descent_tas(10_000.0, None, 500.0), 290.0);
}

// AC 3. A flat record (descent 2,000 fpm, 240 kt; envelope 400 to 500): from 30,000 ft to sea level
// 15 min x 4 nm/min = 60 nm; to a 6,000 ft field 12 min = 48 nm; from 0 to 0 = 0. Bundled B738 FL350 to 0:
// 130.884 nm. Red when: the field elevation is ignored (60, not 48).
#[test]
fn the_descent_distance_integrates_the_descent_table_to_the_field() {
    let r = flat(300.0, 240.0);
    let p = Performance::Eram(&r);
    for (from, to, want) in [
        (30_000.0, 0.0, 60.0),
        (30_000.0, 6_000.0, 48.0),
        (0.0, 0.0, 0.0),
    ] {
        let nm = p.descent_distance_nm(from, to, 480.0);
        assert!(close(nm, want, 1e-9), "{from} {to}: {nm}");
    }
    // The file's own table, whatever the flight filed.
    assert!(close(
        p.descent_distance_nm(30_000.0, 0.0, 200.0),
        60.0,
        1e-9
    ));
}

// AC 10. The flat record (climb 2,000 fpm, 300 kt): to 30,000 ft 15 min x 5 nm/min = 75 nm; to 10,000 ft
// 25 nm. Bundled B738 to FL350: 105.884 nm. Always from the surface, not the departure field.
// Red when: it is measured from the present altitude.
#[test]
fn the_climb_distance_integrates_the_climb_table_from_the_surface() {
    let r = flat(300.0, 240.0);
    let p = Performance::Eram(&r);
    assert!(close(p.climb_distance_nm(30_000.0), 75.0, 1e-9));
    assert!(close(p.climb_distance_nm(10_000.0), 25.0, 1e-9));
    assert_eq!(p.climb_distance_nm(0.0), 0.0);
}

// AC 3, AC 10. The default row (250/290 KIAS, 2,000/1,500 fpm up, 1,800 down, cruise 440 at FL350):
// descent distance 118.80 nm (the 440 kt cap applies above ~26,600 ft), climb distance 139.00 nm, in
// 500 ft steps on ISA; allow 0.1. Both are still-air distances: wind moves neither.
#[test]
fn a_rows_distances_integrate_its_schedule() {
    let default = AircraftProfile::default();
    let p = Performance::Row(&default);
    let down = p.descent_distance_nm(35_000.0, 0.0, 440.0);
    assert!(close(down, 118.80, 0.1), "{down}");
    let up = p.climb_distance_nm(35_000.0);
    assert!(close(up, 139.00, 0.1), "{up}");
    // A stored rate of 0 is a long way, not an infinite one.
    let stuck = AircraftProfile {
        climb_fpm_lo: 0.0,
        desc_fpm: 0.0,
        ..AircraftProfile::default()
    };
    let p = Performance::Row(&stuck);
    assert!(p.climb_distance_nm(5_000.0).is_finite());
    assert!(p.climb(5_000.0, None).1 > 0.0);
    assert!(p.descent_distance_nm(5_000.0, 0.0, 440.0).is_finite());
}

// AC 3. A flat record with descent 520 kt, cruise filed 480: descent TAS 480 at 13,000 ft. Descent 460 at
// 1.21 x ISA is 506 scaled -> 480. Default row at FL350, cruise 440: 440, not 521. A C208-like row
// (140/110 KIAS, cruise 170) at 10,000 ft: 162.9, below cruise, untouched (control).
// Red when: the min is dropped, or applied before the temperature scale.
#[test]
fn a_descent_speed_is_held_to_the_cruise_speed() {
    let fast = flat(300.0, 520.0);
    assert_eq!(
        Performance::Eram(&fast).descent_tas(13_000.0, None, 480.0),
        480.0
    );
    let warm = flat(300.0, 460.0);
    let temp = Some(isa_temp_k(13_000.0) * 1.21);
    assert_eq!(
        Performance::Eram(&warm).descent_tas(13_000.0, temp, 480.0),
        480.0
    );
    assert_eq!(
        Performance::Eram(&warm).descent_tas(13_000.0, None, 480.0),
        460.0
    );
    let default = AircraftProfile::default();
    assert_eq!(
        Performance::Row(&default).descent_tas(35_000.0, None, 440.0),
        440.0
    );
    let slow = AircraftProfile {
        desc_ias_hi: 140.0,
        desc_ias_lo: 110.0,
        ..AircraftProfile::default()
    };
    let tas = Performance::Row(&slow).descent_tas(10_000.0, None, 170.0);
    assert!(close(tas, 162.9, 0.1), "{tas}");
}

// AC 9. tas_to_cas(ias_to_tas(280, a), a) = 280 within 1e-9 at 0, 10,000, 24,000 and 39,000 ft, and
// tas_to_cas(290.9, 10_000) = 250.0 +/- 0.1, a literal side. Red when: the density ratio is not rooted.
#[test]
fn tas_to_cas_inverts_ias_to_tas() {
    for alt in [0.0, 10_000.0, 24_000.0, 39_000.0] {
        let cas = tas_to_cas(ias_to_tas(280.0, alt), alt);
        assert!(close(cas, 280.0, 1e-9), "{alt}: {cas}");
    }
    let cas = tas_to_cas(290.9, 10_000.0);
    assert!(close(cas, 250.0, 0.1), "{cas}");
}
