//! Sector occupancy (#721, epic #720): per sector, per 15-minute bin over six hours, the **peak
//! one-minute concurrent occupancy** — the busiest minute's count of distinct flights inside the sector —
//! not the number that pass through (vTBFM manual §10). Active and proposed are counted apart.
//!
//! Pure and DB-free like the rest of `feed`: the caller hands in the cached sector table
//! (`AppState::airspace_sectors`) and the flights projected minute by minute
//! ([`sector_tracks::project_tracks`](super::sector_tracks::project_tracks)); this only bins.
//!
//! **Why not `gdp::demand_bins`.** That function *sums events* — each assignment's CTA lands in one bin
//! — and colours the sum against an AAR. A sector cell is the *maximum over the bin's minutes of a
//! distinct-flight set's size*. They share the bin width ([`BIN_MIN`]) and nothing else.
//!
//! Re-landed from the removed Airspace Monitor's binner (#597, removed in #719) without its limit,
//! consolidation and staffing coupling, which #722 and #723 rebuild on top of this.

use std::collections::{BTreeMap, HashMap, HashSet};

pub use crate::feed::gdp::BIN_MIN;
use crate::feed::sectors::SectorTable;

/// Every sector is computed for the full six hours, whatever a view draws.
pub const HORIZON_MIN: i64 = 6 * 60;
const MINUTE_MS: i64 = 60_000;

/// Which count a flight feeds. A flight is in exactly one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Population {
    /// Airborne (>= 50 kt), integrated from the latest poll.
    Active,
    /// On the ground holding a locked wheels-up, integrated forward from it.
    Proposed,
}

/// One predicted position. Any fix inside a sector counts its whole minute for that sector, so the
/// caller may sample as finely as it likes. `alt_ft = None` (unknown) counts laterally only.
#[derive(Debug, Clone, Copy)]
pub struct Fix {
    pub t_ms: i64,
    pub lat: f64,
    pub lon: f64,
    pub alt_ft: Option<f64>,
}

/// One flight's predicted path.
pub struct Track<'a> {
    pub id: &'a str,
    pub population: Population,
    pub fixes: &'a [Fix],
}

/// A bin's peak one-minute counts. `combined` is maxed minute by minute — never `active + proposed`,
/// whose busiest minutes can differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BinPeak {
    pub start_ms: i64,
    pub active: usize,
    pub proposed: usize,
    pub combined: usize,
}

/// One sector's row: [`HORIZON_MIN`] / [`BIN_MIN`] bins, the first being the Zulu quarter-hour that
/// contains `now`. A sector stored as several volumes is one row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectorLoad {
    pub artcc: String,
    pub sector_id: String,
    /// The sector's stratum (`low`, `high`, `ultra_high`, `approach`), from its first volume.
    pub tier: String,
    pub bins: Vec<BinPeak>,
}

/// Peak occupancy for every sector in `table`, sorted by `(artcc, sector_id)`.
///
/// Bins are absolute Zulu quarter-hours — at 1407Z the first starts at 1400 — so they never depend on
/// when the process started. A flight counts in a sector in a minute when any of its fixes that minute
/// is inside any of the sector's volumes, laterally **and** between the volume's floor and ceiling
/// ([`SectorVolume::contains`](super::sectors::SectorVolume::contains)). Occupants are sets keyed by
/// sector and minute, so a boundary skim or a crossing between a sector's pieces counts once.
pub fn sector_loads(table: &SectorTable, tracks: &[Track], now_ms: i64) -> Vec<SectorLoad> {
    let bin_ms = BIN_MIN * MINUTE_MS;
    let first_ms = now_ms - now_ms.rem_euclid(bin_ms);
    let end_ms = first_ms + HORIZON_MIN * MINUTE_MS;

    // Each sector once, with its tier, and the row every volume counts under.
    let mut sectors: BTreeMap<(&str, &str), &str> = BTreeMap::new();
    for v in &table.volumes {
        sectors
            .entry((v.artcc.as_str(), v.sector_id.as_str()))
            .or_insert(v.tier.as_str());
    }
    let row: HashMap<(&str, &str), usize> = sectors
        .keys()
        .enumerate()
        .map(|(i, key)| (*key, i))
        .collect();
    let row_of_volume: Vec<usize> = table
        .volumes
        .iter()
        .map(|v| row[&(v.artcc.as_str(), v.sector_id.as_str())])
        .collect();

    // Who is inside each sector in each minute, by population. Sparse: most sector-minutes are empty.
    type Occupants<'a> = (HashSet<&'a str>, HashSet<&'a str>);
    let mut occupied: HashMap<(usize, i64), Occupants> = HashMap::new();
    for track in tracks {
        for fix in track
            .fixes
            .iter()
            .filter(|f| (first_ms..end_ms).contains(&f.t_ms))
        {
            let minute = (fix.t_ms - first_ms) / MINUTE_MS;
            let inside = table
                .volumes
                .iter()
                .enumerate()
                .filter(|(_, v)| v.contains(fix.lat, fix.lon, fix.alt_ft));
            for (i, _) in inside {
                let (active, proposed) = occupied.entry((row_of_volume[i], minute)).or_default();
                match track.population {
                    Population::Active => active.insert(track.id),
                    Population::Proposed => proposed.insert(track.id),
                };
            }
        }
    }

    sectors
        .iter()
        .enumerate()
        .map(|(sector, ((artcc, sector_id), tier))| SectorLoad {
            artcc: artcc.to_string(),
            sector_id: sector_id.to_string(),
            tier: tier.to_string(),
            bins: (0..HORIZON_MIN / BIN_MIN)
                .map(|bin| {
                    let mut peak = BinPeak {
                        start_ms: first_ms + bin * bin_ms,
                        active: 0,
                        proposed: 0,
                        combined: 0,
                    };
                    for minute in bin * BIN_MIN..(bin + 1) * BIN_MIN {
                        if let Some((a, p)) = occupied.get(&(sector, minute)) {
                            peak.active = peak.active.max(a.len());
                            peak.proposed = peak.proposed.max(p.len());
                            // Distinct, because a flight is in exactly one population.
                            peak.combined = peak.combined.max(a.len() + p.len());
                        }
                    }
                    peak
                })
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::feed::sectors::{SectorVolume, tests::volume};

    /// 1407Z on an arbitrary day; the first bin starts at 1400.
    fn now() -> i64 {
        at(14, 7, 0)
    }
    fn at(h: u32, m: u32, s: u32) -> i64 {
        Utc.with_ymd_and_hms(2026, 10, 5, h, m, s)
            .unwrap()
            .timestamp_millis()
    }

    /// Inside the fixture square (38–39N, 76–77W), at an altitude inside its 0–23,000 ft band.
    fn inside(t_ms: i64) -> Fix {
        Fix {
            t_ms,
            lat: 38.5,
            lon: -76.5,
            alt_ft: Some(10_000.0),
        }
    }
    fn outside(t_ms: i64) -> Fix {
        Fix {
            lat: 40.0,
            ..inside(t_ms)
        }
    }
    fn at_alt(t_ms: i64, alt_ft: f64) -> Fix {
        Fix {
            alt_ft: Some(alt_ft),
            ..inside(t_ms)
        }
    }

    fn one_sector() -> SectorTable {
        SectorTable {
            volumes: vec![volume("ZDC", "01001")],
        }
    }

    fn active<'a>(id: &'a str, fixes: &'a [Fix]) -> Track<'a> {
        Track {
            id,
            population: Population::Active,
            fixes,
        }
    }

    /// The first bin of the only sector.
    fn first_bin(table: &SectorTable, tracks: &[Track]) -> BinPeak {
        sector_loads(table, tracks, now())[0].bins[0]
    }

    /// AC1: 40 flights transit the sector within one quarter-hour, each inside for one minute and never
    /// more than 3 at once. The cell is 3 (peak concurrency), not 40 (throughput).
    #[test]
    fn the_cell_is_peak_concurrency_not_throughput() {
        let ids: Vec<String> = (0..40).map(|i| format!("F{i:02}")).collect();
        let fixes: Vec<[Fix; 1]> = (0..40u32)
            .map(|i| [inside(at(14, i * 15 / 40, 30))])
            .collect();
        let tracks: Vec<Track> = ids
            .iter()
            .zip(&fixes)
            .map(|(id, f)| active(id, f))
            .collect();

        let transits: HashSet<&str> = tracks.iter().map(|t| t.id).collect();
        assert_eq!(transits.len(), 40, "throughput in the bin is 40");
        assert_eq!(first_bin(&one_sector(), &tracks).active, 3);
    }

    /// AC2: in, out and back in within one minute is one flight in that minute.
    #[test]
    fn a_boundary_skim_within_a_minute_counts_once() {
        let fixes = [
            inside(at(14, 1, 10)),
            outside(at(14, 1, 30)),
            inside(at(14, 1, 50)),
        ];
        assert_eq!(
            first_bin(&one_sector(), &[active("SKIM", &fixes)]).active,
            1
        );
    }

    /// AC3: an approach volume (0–11,000 ft) under an enroute high (18,000–60,000 ft) over one footprint.
    /// A flight at FL240 counts in the enroute sector and **not** the TRACON; one at 8,000 ft the
    /// reverse. Both volumes are asserted in the same test.
    #[test]
    fn a_flight_at_fl240_over_a_tracon_counts_enroute_not_approach() {
        let tracon = SectorVolume {
            sector_id: "TRA".into(),
            tier: "approach".into(),
            base_alt_ft: 0,
            top_alt_ft: 11_000,
            ..volume("ZDC", "TRA01")
        };
        let enroute = SectorVolume {
            sector_id: "H24".into(),
            tier: "high".into(),
            base_alt_ft: 18_000,
            top_alt_ft: 60_000,
            ..volume("ZDC", "H2401")
        };
        let table = SectorTable {
            volumes: vec![tracon, enroute],
        };
        let high = [at_alt(at(14, 2, 0), 24_000.0)];
        let low = [at_alt(at(14, 9, 0), 8_000.0)];

        let fl240 = sector_loads(&table, &[active("HIGH", &high)], now());
        let row = |loads: &[SectorLoad], id: &str| {
            loads.iter().find(|l| l.sector_id == id).unwrap().bins[0].active
        };
        assert_eq!(row(&fl240, "H24"), 1, "FL240 is in the enroute sector");
        assert_eq!(row(&fl240, "TRA"), 0, "and not in the TRACON beneath it");

        let at_8000 = sector_loads(&table, &[active("LOW", &low)], now());
        assert_eq!(row(&at_8000, "TRA"), 1, "8,000 ft is in the TRACON");
        assert_eq!(
            row(&at_8000, "H24"),
            0,
            "and not in the enroute sector above it"
        );
    }

    /// AC4: a sector stored as two volumes is one row, and a flight in both pieces in one minute is one.
    #[test]
    fn a_sector_of_several_volumes_counts_a_flight_once() {
        let mut second = volume("ZDC", "01002");
        second.sector_id = "010".into(); // same sector as `volume("ZDC", "01001")`
        let table = SectorTable {
            volumes: vec![volume("ZDC", "01001"), second],
        };
        let fixes = [inside(at(14, 2, 0))];
        let loads = sector_loads(&table, &[active("A", &fixes)], now());
        assert_eq!(loads.len(), 1);
        assert_eq!(loads[0].bins[0].active, 1);
    }

    /// AC5: bins are absolute quarter-hours — at 1407Z the first is 1400 — over six hours, and any two
    /// moments inside one quarter-hour produce the same bins.
    #[test]
    fn bins_align_to_the_zulu_quarter_hour() {
        let fixes = [
            inside(at(13, 59, 0)),  // before the first bin: ignored
            inside(at(14, 14, 59)), // last minute of bin 0
            inside(at(14, 15, 0)),  // first minute of bin 1
            inside(at(20, 0, 0)),   // six hours after 1400: past the horizon
        ];
        let row = &sector_loads(&one_sector(), &[active("A", &fixes)], now())[0];
        assert_eq!(row.bins.len(), 24);
        assert_eq!(row.bins[0].start_ms, at(14, 0, 0));
        assert_eq!(row.bins[1].start_ms, at(14, 15, 0));
        let counts: Vec<usize> = row.bins.iter().map(|b| b.active).collect();
        assert_eq!(&counts[..2], [1, 1]);
        assert_eq!(counts[2..].iter().sum::<usize>(), 0);

        let starts = |now_ms| -> Vec<i64> {
            sector_loads(&one_sector(), &[], now_ms)[0]
                .bins
                .iter()
                .map(|b| b.start_ms)
                .collect()
        };
        assert_eq!(
            starts(at(14, 0, 1)),
            starts(at(14, 14, 59)),
            "same quarter-hour, same bins"
        );
    }

    /// AC6: active and proposed are returned apart. The active peak (2, minute 0) and the proposed peak
    /// (3, minute 5) fall in different minutes, so the combined peak is the busiest single minute (4),
    /// not the sum of the peaks (5).
    #[test]
    fn active_and_proposed_are_separate_and_combined_is_maxed_by_minute() {
        let m0 = [inside(at(14, 0, 30))];
        let m0_m5 = [inside(at(14, 0, 30)), inside(at(14, 5, 30))];
        let m5 = [inside(at(14, 5, 30))];
        let track = |id, population, fixes| Track {
            id,
            population,
            fixes,
        };
        let tracks = [
            track("A1", Population::Active, &m0_m5[..]),
            track("A2", Population::Active, &m0[..]),
            track("P1", Population::Proposed, &m0_m5[..]),
            track("P2", Population::Proposed, &m5[..]),
            track("P3", Population::Proposed, &m5[..]),
        ];
        let peak = first_bin(&one_sector(), &tracks);
        assert_eq!((peak.active, peak.proposed, peak.combined), (2, 3, 4));
    }

    /// A sector with no traffic still gets its row of zeros, with its tier.
    #[test]
    fn an_empty_sector_still_has_a_row() {
        let loads = sector_loads(&one_sector(), &[], now());
        assert_eq!(loads.len(), 1);
        assert_eq!(loads[0].tier, "low");
        assert!(loads[0].bins.iter().all(|b| b.combined == 0));
    }
}
