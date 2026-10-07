//! TRACON strata (#726): vertical containment and TRACON precedence, through [`sector_loads`].
//!
//! Every test reads the rows `sector_loads` returns, never `SectorTable::counting` directly, so it
//! pins what an occupancy cell shows. The synthetic tests stack volumes on the fixture square
//! (38–39N, 76–77W). The `#[ignore]`d one runs the rule over the real vTSD table, fetched at the
//! importer's pinned commit and never committed (the source is CC BY-NC-SA, OIS is MIT).

use std::collections::{BTreeMap, BTreeSet};

use chrono::{TimeZone, Utc};

use super::*;
use crate::feed::sectors::{APPROACH_TIER, SectorVolume, tests::volume};

/// 1407Z on an arbitrary day; the first bin starts at 1400.
fn now() -> i64 {
    Utc.with_ymd_and_hms(2026, 10, 5, 14, 7, 0)
        .unwrap()
        .timestamp_millis()
}

/// A volume on the fixture square with an explicit sector, tier and band.
fn vol(
    artcc: &str,
    volume_id: &str,
    sector_id: &str,
    tier: &str,
    base: i32,
    top: i32,
) -> SectorVolume {
    SectorVolume {
        sector_id: sector_id.into(),
        tier: tier.into(),
        base_alt_ft: base,
        top_alt_ft: top,
        ..volume(artcc, volume_id)
    }
}

/// A fix in the first minute at `(lat, lon, alt_ft)`.
fn fix_at(lat: f64, lon: f64, alt_ft: Option<f64>) -> Fix {
    Fix {
        t_ms: now() + 30_000,
        lat,
        lon,
        alt_ft,
    }
}

/// The centre of the fixture square, which is also inside [`inner_tracon`]'s smaller ring.
fn centre(alt_ft: Option<f64>) -> Fix {
    fix_at(38.5, -76.5, alt_ft)
}

/// Every row's first-bin active count, keyed `ARTCC/sector` — every row, so an assertion against the
/// whole map also proves no other sector counted the flight.
fn first_bins(table: &SectorTable, fixes: &[Fix]) -> BTreeMap<String, usize> {
    let track = Track {
        id: "A",
        population: Population::Active,
        fixes,
    };
    sector_loads(table, &[track], now())
        .into_iter()
        .map(|l| (format!("{}/{}", l.artcc, l.sector_id), l.bins[0].active))
        .collect()
}

fn expect(rows: &[(&str, usize)]) -> BTreeMap<String, usize> {
    rows.iter().map(|(k, n)| (k.to_string(), *n)).collect()
}

/// A TRACON (0–10,000), an enroute Low starting at the surface (0–23,000) and a High (23,000–60,000),
/// all on one footprint — the real shape: the enroute Low sits laterally over the TRACON.
fn stack() -> SectorTable {
    SectorTable {
        volumes: vec![
            vol("ZDC", "07001", "APP", "approach", 0, 10_000),
            vol("ZDC", "05901", "LOW", "low", 0, 23_000),
            vol("ZDC", "01001", "HIG", "high", 23_000, 60_000),
        ],
    }
}

/// AC1: FL240 over a TRACON counts in the enroute volume and **not** the approach volume — every row
/// asserted at once.
#[test]
fn fl240_over_a_tracon_counts_enroute_not_approach() {
    assert_eq!(
        first_bins(&stack(), &[centre(Some(24_000.0))]),
        expect(&[("ZDC/APP", 0), ("ZDC/HIG", 1), ("ZDC/LOW", 0)]),
    );
}

/// AC2: 8,000 ft inside the TRACON counts in the approach volume only — not the enroute Low whose band
/// also holds 8,000 (precedence), nor the High above.
#[test]
fn eight_thousand_inside_a_tracon_counts_approach_only() {
    assert_eq!(
        first_bins(&stack(), &[centre(Some(8_000.0))]),
        expect(&[("ZDC/APP", 1), ("ZDC/HIG", 0), ("ZDC/LOW", 0)]),
    );
}

/// AC3: at a shared boundary a fix is in exactly one volume. The issue's case first — approach top
/// 18,000 on enroute base 18,000 — at boundary − 1, boundary and boundary + 1. Then the TRACON top
/// under an overlapping Low: precedence holds only while the fix is *in* the approach band, so it
/// releases at the top.
#[test]
fn a_shared_boundary_counts_in_exactly_one_volume() {
    let issue = SectorTable {
        volumes: vec![
            vol("ZDC", "07001", "APP", "approach", 0, 18_000),
            vol("ZDC", "01001", "HIG", "high", 18_000, 60_000),
        ],
    };
    for (alt, rows) in [
        (17_999.0, [("ZDC/APP", 1), ("ZDC/HIG", 0)]),
        (18_000.0, [("ZDC/APP", 0), ("ZDC/HIG", 1)]),
        (18_001.0, [("ZDC/APP", 0), ("ZDC/HIG", 1)]),
    ] {
        let got = first_bins(&issue, &[centre(Some(alt))]);
        assert_eq!(got, expect(&rows), "at {alt} ft");
        assert_eq!(got.values().filter(|n| **n > 0).count(), 1, "at {alt} ft");
    }

    for (alt, rows) in [
        (9_999.0, [("ZDC/APP", 1), ("ZDC/HIG", 0), ("ZDC/LOW", 0)]),
        (10_000.0, [("ZDC/APP", 0), ("ZDC/HIG", 0), ("ZDC/LOW", 1)]),
        (10_001.0, [("ZDC/APP", 0), ("ZDC/HIG", 0), ("ZDC/LOW", 1)]),
    ] {
        let got = first_bins(&stack(), &[centre(Some(alt))]);
        assert_eq!(got, expect(&rows), "at {alt} ft, the TRACON top");
        assert_eq!(got.values().filter(|n| **n > 0).count(), 1, "at {alt} ft");
    }
}

/// AC4: one sector stored as two volumes in different strata (0–11,000 and 11,000–23,000). A flight
/// with a fix in each piece in one minute is one flight, in one row.
#[test]
fn a_sector_split_across_strata_counts_a_flight_once() {
    let table = SectorTable {
        volumes: vec![
            vol("ZDC", "05901", "LOW", "low", 0, 11_000),
            vol("ZDC", "05902", "LOW", "low", 11_000, 23_000),
        ],
    };
    let fixes = [
        centre(Some(9_000.0)),
        Fix {
            t_ms: now() + 45_000,
            ..centre(Some(15_000.0))
        },
    ];
    assert_eq!(first_bins(&table, &fixes), expect(&[("ZDC/LOW", 1)]));
}

/// A TRACON on a smaller ring (38.2–38.8N, 76.2–76.8W) inside the fixture square, 0–10,000, under the
/// Low and High of [`stack`].
fn inner_tracon() -> SectorTable {
    let mut table = stack();
    table.volumes[0].rings = vec![vec![
        [38.2, -76.8],
        [38.2, -76.2],
        [38.8, -76.2],
        [38.8, -76.8],
        [38.2, -76.8],
    ]];
    table
}

/// Precedence is lateral too: at 8,000 the Low counts a fix just outside the TRACON's ring, and the
/// TRACON (not the Low) counts one just inside it.
#[test]
fn outside_the_tracon_ring_the_enroute_low_counts() {
    let table = inner_tracon();
    assert_eq!(
        first_bins(&table, &[fix_at(38.1, -76.5, Some(8_000.0))]),
        expect(&[("ZDC/APP", 0), ("ZDC/HIG", 0), ("ZDC/LOW", 1)]),
        "outside the ring",
    );
    assert_eq!(
        first_bins(&table, &[fix_at(38.3, -76.5, Some(8_000.0))]),
        expect(&[("ZDC/APP", 1), ("ZDC/HIG", 0), ("ZDC/LOW", 0)]),
        "inside the ring",
    );
}

/// Precedence is global: a ZJX approach volume (3,000–5,000) suppresses the ZMA Low over it, as
/// `ZJX approach 00/00022` does to `ZMA Ultra High 02/00201` in the real data. Above its top, ZMA's
/// Low counts again.
#[test]
fn an_approach_volume_takes_precedence_across_artccs() {
    let table = SectorTable {
        volumes: vec![
            vol("ZJX", "00022", "00", "approach", 3_000, 5_000),
            vol("ZMA", "00201", "02", "low", 0, 23_900),
        ],
    };
    assert_eq!(
        first_bins(&table, &[centre(Some(4_000.0))]),
        expect(&[("ZJX/00", 1), ("ZMA/02", 0)]),
    );
    assert_eq!(
        first_bins(&table, &[centre(Some(6_000.0))]),
        expect(&[("ZJX/00", 0), ("ZMA/02", 1)]),
    );
}

/// An unknown altitude fails open laterally, so it is a candidate in every stratum; inside a TRACON's
/// ring the TRACON alone counts it. Outside the ring there is no approach candidate, so every
/// stratum counts it, as before #726.
#[test]
fn an_unknown_altitude_inside_a_tracon_counts_approach_only() {
    let table = inner_tracon();
    assert_eq!(
        first_bins(&table, &[centre(None)]),
        expect(&[("ZDC/APP", 1), ("ZDC/HIG", 0), ("ZDC/LOW", 0)]),
    );
    assert_eq!(
        first_bins(&table, &[fix_at(38.1, -76.5, None)]),
        expect(&[("ZDC/APP", 0), ("ZDC/HIG", 1), ("ZDC/LOW", 1)]),
    );
}

/// ZSE's source data has no Approach Control volumes (a gap in vTSD `sectors.json`, not a real
/// absence). `sector_loads` keeps that distinct from a quiet TRACON: ZSE gets no `approach` row at
/// all, while ZTL's TRACON with no traffic still has its row, every bin zero.
#[test]
fn a_facility_without_tracon_data_is_distinct_from_a_quiet_tracon() {
    let table = SectorTable {
        volumes: vec![
            vol("ZSE", "02001", "20", "low", 0, 23_000),
            vol("ZSE", "05001", "50", "ultra_high", 23_000, 60_000),
            vol("ZTL", "07001", "70", "approach", 0, 4_000),
        ],
    };
    let loads = sector_loads(&table, &[], now());

    let zse: Vec<&str> = loads
        .iter()
        .filter(|l| l.artcc == "ZSE")
        .map(|l| l.tier.as_str())
        .collect();
    assert_eq!(
        zse,
        ["low", "ultra_high"],
        "ZSE has rows, none of them approach"
    );

    let ztl: Vec<&SectorLoad> = loads
        .iter()
        .filter(|l| l.artcc == "ZTL" && l.tier == APPROACH_TIER)
        .collect();
    assert_eq!(ztl.len(), 1, "the quiet TRACON keeps its row");
    assert_eq!(ztl[0].sector_id, "70");
    assert!(
        ztl[0]
            .bins
            .iter()
            .all(|b| b.active == 0 && b.proposed == 0 && b.combined == 0)
    );
}

/// The real vTSD table, parsed as the importer does.
mod real {
    use serde::Deserialize;

    use crate::feed::sectors::{SectorTable, SectorVolume, validate_volume};

    /// Must equal `SOURCE_REF` in `bin/airspace_sector_importer.rs`, so this test checks the data
    /// production actually imports. Move them together.
    const SOURCE_REF: &str = "f33ef73f21091e71456e45cfe9019ddf3ba76247";

    #[derive(Deserialize)]
    struct Collection {
        features: Vec<Feature>,
    }

    #[derive(Deserialize)]
    struct Feature {
        properties: Props,
        geometry: Geometry,
    }

    #[derive(Deserialize)]
    struct Props {
        artcc: String,
        sector: String,
        tier: String,
        base_alt: i32,
        max_alt: i32,
        full_id: String,
    }

    /// GeoJSON positions are `[lon, lat, ...]`.
    #[derive(Deserialize)]
    #[serde(tag = "type", content = "coordinates")]
    enum Geometry {
        Polygon(Vec<Vec<Vec<f64>>>),
        MultiPolygon(Vec<Vec<Vec<Vec<f64>>>>),
    }

    /// A test-local mirror of the importer's `to_volume`: tier mapped, positions flipped to
    /// `[lat, lon]`, one ring per polygon part, holes refused.
    fn to_volume(f: Feature) -> Result<SectorVolume, String> {
        let p = f.properties;
        let tier = match p.tier.as_str() {
            "Low" => "low",
            "High" => "high",
            "Ultra High" => "ultra_high",
            "Approach Control" => "approach",
            other => return Err(format!("unknown tier {other:?}")),
        };
        let polygons = match f.geometry {
            Geometry::Polygon(rings) => vec![rings],
            Geometry::MultiPolygon(polys) => polys,
        };
        let rings = polygons
            .into_iter()
            .map(|poly| match poly.as_slice() {
                [ring] => ring
                    .iter()
                    .map(|pos| match pos.as_slice() {
                        [lon, lat, ..] => Ok([*lat, *lon]),
                        _ => Err("position has fewer than two numbers".to_string()),
                    })
                    .collect(),
                _ => Err(format!("polygon has {} rings", poly.len())),
            })
            .collect::<Result<_, _>>()?;
        Ok(SectorVolume {
            artcc: p.artcc.to_ascii_uppercase(),
            sector_id: p.sector,
            volume_id: p.full_id,
            name: None,
            tier: tier.into(),
            base_alt_ft: p.base_alt,
            top_alt_ft: p.max_alt,
            rings,
        })
    }

    /// The table the importer would store, and the `ARTCC full_id` of every feature it would skip.
    pub(super) async fn table() -> (SectorTable, Vec<String>) {
        let url = format!(
            "https://raw.githubusercontent.com/Virtual-Traffic-Situation-Display/vtsd/{SOURCE_REF}/Data/sectors.json"
        );
        let collection: Collection = reqwest::Client::builder()
            .user_agent("ois-airspace-sector-importer/0.1 (+https://vatusa.net)")
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap()
            .get(&url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .unwrap_or_else(|e| panic!("fetching {url}: {e}"))
            .json()
            .await
            .unwrap_or_else(|e| panic!("parsing {url}: {e}"));

        let mut volumes = Vec::new();
        let mut skipped = Vec::new();
        for f in collection.features {
            let id = format!("{} {}", f.properties.artcc, f.properties.full_id);
            match to_volume(f).and_then(|v| validate_volume(&v).map(|()| v)) {
                Ok(v) => volumes.push(v),
                Err(_) => skipped.push(id),
            }
        }
        (SectorTable { volumes }, skipped)
    }
}

/// The rule over the whole real table: each fix lands in exactly the expected sectors (every one of
/// the ~1,500 volumes is a candidate), ZTL `07007` stays skipped, and ZSE has no TRACON data.
#[tokio::test]
#[ignore = "fetches the pinned vTSD sectors.json over the network"]
async fn the_real_table_counts_each_fix_in_its_expected_stratum() {
    let (table, skipped) = real::table().await;
    assert!(
        table.volumes.len() >= 1_500,
        "{} volumes",
        table.volumes.len()
    );

    let counted = |lat: f64, lon: f64, alt: f64| -> BTreeSet<(String, String)> {
        let fixes = [fix_at(lat, lon, Some(alt))];
        let track = Track {
            id: "A",
            population: Population::Active,
            fixes: &fixes,
        };
        sector_loads(&table, &[track], now())
            .into_iter()
            .filter(|l| l.bins[0].active > 0)
            .map(|l| (l.artcc, l.sector_id))
            .collect()
    };
    let only =
        |artcc: &str, sector: &str| BTreeSet::from([(artcc.to_string(), sector.to_string())]);
    let contains = |lat, lon, alt, artcc: &str, volume_id: &str| {
        table
            .containing(lat, lon, Some(alt))
            .any(|v| v.artcc == artcc && v.volume_id == volume_id)
    };

    // KATL: the TRACON (70, 0–4,000) under the Low (59/05906, 0–23,000), released at its top.
    let katl = (33.6367, -84.4281);
    assert!(
        contains(katl.0, katl.1, 3_000.0, "ZTL", "05906"),
        "the Low also contains KATL at 3,000, so precedence is what excludes it",
    );
    for (alt, sector) in [
        (3_000.0, "70"),
        (3_999.0, "70"),
        (4_000.0, "59"),
        (8_000.0, "59"),
        (35_000.0, "27"),
    ] {
        assert_eq!(
            counted(katl.0, katl.1, alt),
            only("ZTL", sector),
            "KATL at {alt} ft"
        );
    }

    // Inside A80, away from the field: FL240 over the TRACON is the High (01001), and at 3,000 the
    // TRACON beats Low 09/00901, which also contains the point.
    let a80 = (33.134, -84.892);
    assert!(contains(a80.0, a80.1, 3_000.0, "ZTL", "00901"));
    assert_eq!(
        counted(a80.0, a80.1, 24_000.0),
        only("ZTL", "10"),
        "A80 at FL240"
    );
    assert_eq!(
        counted(a80.0, a80.1, 3_000.0),
        only("ZTL", "70"),
        "A80 at 3,000"
    );

    // ZTL 07007 (floor above ceiling) is skipped and listed, never stored.
    assert!(
        skipped.contains(&"ZTL 07007".to_string()),
        "skipped: {skipped:?}"
    );
    assert!(
        !table
            .volumes
            .iter()
            .any(|v| v.artcc == "ZTL" && v.volume_id == "07007")
    );

    // ZSE: enroute volumes but no TRACON volumes — a source gap. KSEA and KPDX at 3,000 are in no
    // volume at all (the Lows are carved around them), so they count nowhere.
    let zse: Vec<&SectorVolume> = table.volumes.iter().filter(|v| v.artcc == "ZSE").collect();
    assert!(!zse.is_empty());
    assert!(zse.iter().all(|v| v.tier != APPROACH_TIER));
    assert!(counted(47.45, -122.31, 3_000.0).is_empty(), "KSEA at 3,000");
    assert!(counted(45.59, -122.60, 3_000.0).is_empty(), "KPDX at 3,000");
}
