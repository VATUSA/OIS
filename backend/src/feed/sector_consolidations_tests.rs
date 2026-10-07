//! VATUSA/OIS#723: a consolidated row is a union of the airspace, never a sum of the rows, and it is
//! judged against the target's limit.
//!
//! Two side-by-side ZDC sectors share one minute-resolution clock: 018 is the west half of the fixture
//! square, 041 the east half. Limits are literals, never `DEFAULT_LIMIT`.

use chrono::{TimeZone, Utc};

use crate::feed::{
    sector_consolidations::{SectorConsolidations, row_limit, row_of},
    sector_limits::{SectorLimits, SectorLoadLevel, level},
    sector_load::{Fix, Population, SectorLoad, Track, sector_loads},
    sectors::{SectorTable, SectorVolume, tests::volume},
};

/// 1407Z; the first bin starts at 1400.
fn now() -> i64 {
    at(14, 7, 0)
}
fn at(h: u32, m: u32, s: u32) -> i64 {
    Utc.with_ymd_and_hms(2026, 10, 5, h, m, s)
        .unwrap()
        .timestamp_millis()
}

/// A square of the fixture's latitude band between two meridians.
fn strip(artcc: &str, volume_id: &str, west: f64, east: f64, tier: &str) -> SectorVolume {
    SectorVolume {
        tier: tier.into(),
        rings: vec![vec![
            [38.0, west],
            [38.0, east],
            [39.0, east],
            [39.0, west],
            [38.0, west],
        ]],
        ..volume(artcc, volume_id)
    }
}

/// ZDC 018 (west, `high`) and 041 (east, `low`), with 018 listed first so the table order alone would
/// give a merged row 018's tier. ZDC 050 is a third sector far to the north; ZNY 018 sits on top of
/// ZDC 018 so a consolidation keyed without its ARTCC would swallow it.
fn table() -> SectorTable {
    SectorTable {
        volumes: vec![
            strip("ZDC", "01801", -77.0, -76.5, "high"),
            strip("ZDC", "04101", -76.5, -76.0, "low"),
            SectorVolume {
                rings: vec![vec![
                    [45.0, -77.0],
                    [45.0, -76.0],
                    [46.0, -76.0],
                    [46.0, -77.0],
                    [45.0, -77.0],
                ]],
                ..volume("ZDC", "05001")
            },
            strip("ZNY", "01801", -77.0, -76.5, "low"),
        ],
    }
}

fn in_018(t_ms: i64) -> Fix {
    Fix {
        t_ms,
        lat: 38.5,
        lon: -76.75,
        alt_ft: Some(10_000.0),
    }
}
fn in_041(t_ms: i64) -> Fix {
    Fix {
        lon: -76.25,
        ..in_018(t_ms)
    }
}

fn eighteen_at_41() -> SectorConsolidations {
    [(("ZDC".to_string(), "018".to_string()), "041".to_string())].into()
}

fn active<'a>(id: &'a str, fixes: &'a [Fix]) -> Track<'a> {
    Track {
        id,
        population: Population::Active,
        fixes,
    }
}

fn row<'a>(loads: &'a [SectorLoad], artcc: &str, sector_id: &str) -> &'a SectorLoad {
    loads
        .iter()
        .find(|l| l.artcc == artcc && l.sector_id == sector_id)
        .unwrap_or_else(|| panic!("no row {artcc} {sector_id}"))
}

/// The two ZDC rows' first-bin active peaks, apart.
fn apart(tracks: &[Track]) -> (usize, usize) {
    let loads = sector_loads(&table(), &SectorConsolidations::new(), tracks, now());
    (
        row(&loads, "ZDC", "018").bins[0].active,
        row(&loads, "ZDC", "041").bins[0].active,
    )
}

/// AC1: an aircraft crossing from 018 into 041 inside one minute is **one** aircraft in the merged
/// row. Each row alone sees it, so the sum of the two separate rows reads 2 and the combined row
/// reads lower.
#[test]
fn a_crossing_between_consolidated_sectors_within_a_minute_counts_once() {
    let fixes = [in_018(at(14, 1, 10)), in_041(at(14, 1, 40))];
    let tracks = [active("X", &fixes)];

    let (west, east) = apart(&tracks);
    assert_eq!((west, east), (1, 1), "each sector alone sees the flight");

    let merged = sector_loads(&table(), &eighteen_at_41(), &tracks, now());
    assert!(
        !merged
            .iter()
            .any(|l| l.artcc == "ZDC" && l.sector_id == "018"),
        "the source row disappears"
    );
    let combined = row(&merged, "ZDC", "041");
    assert_eq!(
        combined.consolidated,
        ["018"],
        "and is marked on the target"
    );
    assert_eq!(combined.bins[0].active, 1, "the union counts it once");
    assert!(combined.bins[0].active < west + east);
}

/// AC1, the other half of "never a sum": busiest minutes that fall at different times don't add up.
#[test]
fn a_combined_row_reads_lower_than_the_sum_of_its_parts() {
    let (p, q) = ([in_018(at(14, 1, 30))], [in_041(at(14, 5, 30))]);
    let tracks = [active("P", &p), active("Q", &q)];

    let (west, east) = apart(&tracks);
    assert_eq!(west + east, 2);
    let merged = sector_loads(&table(), &eighteen_at_41(), &tracks, now());
    assert_eq!(row(&merged, "ZDC", "041").bins[0].active, 1);
}

/// The control for both tests above: two flights in the two sectors in the **same** minute are two
/// in the union — more than either row alone, so the merged row is a union, not the larger row.
#[test]
fn distinct_flights_in_the_same_minute_both_count() {
    let (p, q) = ([in_018(at(14, 1, 30))], [in_041(at(14, 1, 30))]);
    let tracks = [active("P", &p), active("Q", &q)];

    assert_eq!(apart(&tracks), (1, 1));
    let merged = sector_loads(&table(), &eighteen_at_41(), &tracks, now());
    assert_eq!(row(&merged, "ZDC", "041").bins[0].active, 2);
}

/// AC2: the combined row is judged against the target's limit (5) — not the sum (13) and not the
/// maximum (8). Six aircraft at once are over 5 and under both of the others, so only the target's
/// limit turns the row red.
#[test]
fn the_combined_row_uses_the_targets_limit() {
    let fixes: Vec<[Fix; 1]> = (0..6).map(|_| [in_041(at(14, 2, 0))]).collect();
    let ids: Vec<String> = (0..6).map(|i| format!("F{i}")).collect();
    let tracks: Vec<Track> = ids
        .iter()
        .zip(&fixes)
        .map(|(id, f)| active(id, f))
        .collect();
    let limits: SectorLimits = [
        (("ZDC".to_string(), "018".to_string()), 8),
        (("ZDC".to_string(), "041".to_string()), 5),
    ]
    .into();

    let merged = sector_loads(&table(), &eighteen_at_41(), &tracks, now());
    let combined = row(&merged, "ZDC", "041");
    assert_eq!(combined.bins[0].active, 6);
    assert_eq!(row_limit(&limits, combined), 5);
    assert_eq!(
        level(&combined.bins[0], row_limit(&limits, combined)),
        SectorLoadLevel::Over
    );
    for (wrong, why) in [(13, "the sum"), (8, "the maximum")] {
        assert_eq!(
            level(&combined.bins[0], wrong),
            SectorLoadLevel::Ok,
            "{why} would hide it"
        );
    }
}

/// The merged row keeps the target's own tier even though the source's volume comes first in the
/// table; unconsolidated sectors and another ARTCC's same-numbered sector keep their own rows.
#[test]
fn only_the_named_source_is_filed_under_the_target() {
    let fixes = [in_018(at(14, 3, 0))];
    let tracks = [active("X", &fixes)];
    let merged = sector_loads(&table(), &eighteen_at_41(), &tracks, now());

    let ids: Vec<(&str, &str)> = merged
        .iter()
        .map(|l| (l.artcc.as_str(), l.sector_id.as_str()))
        .collect();
    assert_eq!(ids, [("ZDC", "041"), ("ZDC", "050"), ("ZNY", "018")]);
    assert_eq!(row(&merged, "ZDC", "041").tier, "low", "the target's tier");
    assert_eq!(row(&merged, "ZDC", "041").bins[0].active, 1);
    assert!(row(&merged, "ZDC", "050").consolidated.is_empty());
    let zny = row(&merged, "ZNY", "018");
    assert!(zny.consolidated.is_empty());
    assert_eq!(zny.bins[0].active, 1, "ZNY's 018 still counts on its own");
}

#[test]
fn row_of_resolves_one_artccs_source_only() {
    let c = eighteen_at_41();
    assert_eq!(row_of(&c, "ZDC", "018"), "041");
    assert_eq!(row_of(&c, "ZDC", "041"), "041", "a target is its own row");
    assert_eq!(row_of(&c, "ZDC", "050"), "050");
    assert_eq!(row_of(&c, "ZNY", "018"), "018", "the same id elsewhere");
}

/// TRACON precedence (#726) and the union compose: an approach sector worked at an enroute Low (or
/// the reverse) still counts a flight once. ZDC 070 is a TRACON over the west half to 10,000; ZDC 041
/// is a Low over the whole square. A flight at 8,000 leaves the TRACON eastward inside one minute: the
/// TRACON alone counts its west fix (precedence keeps the Low out), the Low its east fix, and the
/// combined row counts one flight, whichever sector is the target.
#[test]
fn an_approach_sector_worked_at_an_enroute_one_counts_a_flight_once() {
    let table = SectorTable {
        volumes: vec![
            SectorVolume {
                top_alt_ft: 10_000,
                ..strip("ZDC", "07001", -77.0, -76.5, "approach")
            },
            volume("ZDC", "04101"),
        ],
    };
    let fixes = [
        Fix {
            alt_ft: Some(8_000.0),
            ..in_018(at(14, 1, 10))
        },
        Fix {
            alt_ft: Some(8_000.0),
            ..in_041(at(14, 1, 40))
        },
    ];
    let tracks = [active("X", &fixes)];

    let apart = sector_loads(&table, &SectorConsolidations::new(), &tracks, now());
    assert_eq!(row(&apart, "ZDC", "070").bins[0].active, 1);
    assert_eq!(row(&apart, "ZDC", "041").bins[0].active, 1);

    for (source, target, tier) in [("070", "041", "low"), ("041", "070", "approach")] {
        let c: SectorConsolidations =
            [(("ZDC".to_string(), source.to_string()), target.to_string())].into();
        let merged = sector_loads(&table, &c, &tracks, now());
        assert_eq!(merged.len(), 1, "{source} at {target}");
        let combined = row(&merged, "ZDC", target);
        assert_eq!(
            combined.tier, tier,
            "{source} at {target}: the target's tier"
        );
        assert_eq!(combined.bins[0].active, 1, "{source} at {target}: once");
    }
}
