//! VATUSA/OIS#722: a sector's limit and the level a bin reads against it.
//!
//! Every expected limit here is the literal `10`, never `DEFAULT_LIMIT`: a fixture derived from the
//! constant under test passes for every value of it.

use chrono::{TimeZone, Utc};

use crate::feed::{
    sector_limits::{SectorLimits, SectorLoadLevel, level, limit_for},
    sector_load::{BinPeak, Fix, Population, Track, sector_loads},
    sectors::{SectorTable, tests::volume},
};

fn bin(active: usize, proposed: usize, combined: usize) -> BinPeak {
    BinPeak {
        start_ms: 0,
        active,
        proposed,
        combined,
    }
}

#[test]
fn a_sector_with_no_override_reads_ten() {
    assert_eq!(limit_for(&SectorLimits::new(), "ZDC", "010"), 10);
}

#[test]
fn an_override_belongs_to_its_own_artcc_and_sector_only() {
    let limits: SectorLimits = [(("ZDC".to_string(), "010".to_string()), 14)].into();

    assert_eq!(limit_for(&limits, "ZDC", "010"), 14);
    assert_eq!(limit_for(&limits, "ZDC", "011"), 10, "a neighbour sector");
    assert_eq!(
        limit_for(&limits, "ZNY", "010"),
        10,
        "the same id elsewhere"
    );
}

/// "At" the limit is not over it: one either side of a limit of 10, for both comparisons.
#[test]
fn a_peak_equal_to_the_limit_is_green() {
    assert_eq!(level(&bin(9, 0, 9), 10), SectorLoadLevel::Ok);
    assert_eq!(level(&bin(10, 0, 10), 10), SectorLoadLevel::Ok);
    assert_eq!(level(&bin(0, 10, 10), 10), SectorLoadLevel::Ok);
    assert_eq!(level(&bin(11, 0, 11), 10), SectorLoadLevel::Over);
    assert_eq!(level(&bin(0, 11, 11), 10), SectorLoadLevel::Watch);
}

/// Red is the active peak alone over the limit; over only once proposed traffic joins is yellow.
#[test]
fn red_needs_active_alone_over_the_limit() {
    // Active at the limit, combined past it: preventable, so Watch — never Over.
    assert_eq!(level(&bin(10, 1, 11), 10), SectorLoadLevel::Watch);
    assert_eq!(level(&bin(10, 5, 15), 10), SectorLoadLevel::Watch);
    // Active one past it: locked in, whatever proposed adds.
    assert_eq!(level(&bin(11, 0, 11), 10), SectorLoadLevel::Over);
    assert_eq!(level(&bin(11, 4, 15), 10), SectorLoadLevel::Over);
}

/// The level reads `combined`, the minute-by-minute peak, never `active + proposed`: here the two
/// peaks sum to 11 but never shared a minute.
#[test]
fn the_level_reads_the_combined_peak_not_a_sum() {
    assert_eq!(level(&bin(6, 5, 6), 10), SectorLoadLevel::Ok);
    assert_eq!(level(&bin(10, 1, 10), 10), SectorLoadLevel::Ok);
}

/// A peak too big for an `i32` is still over an `i32` limit — no lossy cast wraps it negative.
#[test]
fn a_huge_peak_does_not_wrap() {
    let huge = i32::MAX as usize + 1;
    assert_eq!(level(&bin(huge, 0, huge), i32::MAX), SectorLoadLevel::Over);
    assert_eq!(level(&bin(0, huge, huge), i32::MAX), SectorLoadLevel::Watch);
}

/// 1407Z; the first bin is 1400–1415.
fn at(m: u32) -> i64 {
    Utc.with_ymd_and_hms(2026, 10, 5, 14, m, 30)
        .unwrap()
        .timestamp_millis()
}

/// Inside `volume`'s square and altitude band.
fn inside(t_ms: i64) -> Fix {
    Fix {
        t_ms,
        lat: 38.5,
        lon: -76.5,
        alt_ft: Some(10_000.0),
    }
}

/// The first bin of the one sector, built by the real binner: two active flights in `active_min`,
/// three proposed in `proposed_min`.
fn first_bin(active_min: u32, proposed_min: u32) -> BinPeak {
    let table = SectorTable {
        volumes: vec![volume("ZDC", "01001")],
    };
    let a = [inside(at(active_min))];
    let p = [inside(at(proposed_min))];
    let track = |id, population, fixes| Track {
        id,
        population,
        fixes,
    };
    let tracks = [
        track("A1", Population::Active, &a[..]),
        track("A2", Population::Active, &a[..]),
        track("P1", Population::Proposed, &p[..]),
        track("P2", Population::Proposed, &p[..]),
        track("P3", Population::Proposed, &p[..]),
    ];
    let loads = sector_loads(&table, &tracks, at(7));
    assert_eq!(loads.len(), 1);
    loads[0].bins[0]
}

/// Active peaks in minute 0, proposed in minute 5: the busiest minute holds 3, not the 5 a sum of
/// peaks claims, so at a limit of 4 the bin is green.
#[test]
fn peaks_in_different_minutes_are_not_summed() {
    let b = first_bin(0, 5);
    assert_eq!((b.active, b.proposed, b.combined), (2, 3, 3));
    assert_eq!(level(&b, 4), SectorLoadLevel::Ok);
    assert_eq!(level(&b, 3), SectorLoadLevel::Ok, "equal to the limit");
    assert_eq!(level(&b, 2), SectorLoadLevel::Watch);
}

/// The control: the same flights sharing a minute do make 5, and that is yellow at 4. Without it the
/// test above could pass on a fixture the binner never counted together.
#[test]
fn peaks_in_the_same_minute_do_combine() {
    let b = first_bin(5, 5);
    assert_eq!((b.active, b.proposed, b.combined), (2, 3, 5));
    assert_eq!(level(&b, 4), SectorLoadLevel::Watch);
    assert_eq!(level(&b, 1), SectorLoadLevel::Over);
}
