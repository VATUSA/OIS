//! Airspace Monitor loading (#597): per sector, per 15-minute bin, the **peak concurrent occupancy**
//! — the busiest minute's count of distinct flights inside the sector — not the number that pass
//! through (vTBFM manual §10).
//!
//! Pure and DB-free, like `gdp.rs`: the caller projects live flights into minute-by-minute [`Fix`]es
//! (the trajectory walk) and says which population each belongs to; this only bins.
//!
//! **Why not `gdp::demand_bins`** (#597 AC6). That function *sums events* — each assignment's CTA
//! lands in one bin — and colours the sum against an AAR. A sector cell is the *maximum over the
//! bin's minutes of a distinct-flight set's size*. The two share the bin width (reused here as
//! [`BIN_MIN`]) and nothing else, so one function computing both would be a mode switch around an
//! otherwise different loop.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::feed::gdp::BIN_MIN;
use crate::feed::sectors::SectorTable;

/// Every sector is computed for the full six hours, whatever the UI shows.
pub const HORIZON_MIN: i64 = 6 * 60;
const MINUTE_MS: i64 = 60_000;

/// Which count a flight feeds. A flight is in exactly one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Population {
    /// Airborne (>= 50 kt).
    Active,
    /// On the ground holding an issued departure slot, integrated from its assigned wheels-up.
    Proposed,
}

/// One predicted position. Any fix inside a sector counts its whole minute for that sector, so the
/// caller may sample as finely as it likes.
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

/// One sector's row: [`HORIZON_MIN`] / [`BIN_MIN`] bins, the first being the quarter-hour that
/// contains `now`. A sector stored as several volumes is one row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectorLoad {
    pub artcc: String,
    pub sector_id: String,
    pub bins: Vec<BinPeak>,
}

/// Peak occupancy for every sector in `table`, sectors sorted by `(artcc, sector_id)`. Bins are
/// absolute Zulu quarter-hours: at 1407Z the first starts at 1400.
pub fn sector_loads(table: &SectorTable, tracks: &[Track], now_ms: i64) -> Vec<SectorLoad> {
    let bin_ms = BIN_MIN * MINUTE_MS;
    let first_ms = now_ms - now_ms.rem_euclid(bin_ms);
    let end_ms = first_ms + HORIZON_MIN * MINUTE_MS;

    let sectors: BTreeMap<(&str, &str), usize> = table
        .volumes
        .iter()
        .map(|v| (v.artcc.as_str(), v.sector_id.as_str()))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .enumerate()
        .map(|(i, key)| (key, i))
        .collect();

    // Who is inside each sector in each minute, by population. Keyed by sector (not volume) and by
    // minute, so a boundary skim or a crossing between a sector's pieces counts the flight once.
    // Sparse: most sector-minutes are empty.
    type Occupants<'a> = (HashSet<&'a str>, HashSet<&'a str>);
    let mut occupied: HashMap<(usize, i64), Occupants> = HashMap::new();
    for track in tracks {
        for fix in track
            .fixes
            .iter()
            .filter(|f| (first_ms..end_ms).contains(&f.t_ms))
        {
            let minute = (fix.t_ms - first_ms) / MINUTE_MS;
            for v in table.containing(fix.lat, fix.lon, fix.alt_ft) {
                let sector = sectors[&(v.artcc.as_str(), v.sector_id.as_str())];
                let (active, proposed) = occupied.entry((sector, minute)).or_default();
                match track.population {
                    Population::Active => active.insert(track.id),
                    Population::Proposed => proposed.insert(track.id),
                };
            }
        }
    }

    sectors
        .into_iter()
        .map(|((artcc, sector_id), sector)| SectorLoad {
            artcc: artcc.to_string(),
            sector_id: sector_id.to_string(),
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
    use crate::feed::sectors::tests::volume;

    /// 1407Z on an arbitrary day; the first bin starts at 1400.
    fn now() -> i64 {
        Utc.with_ymd_and_hms(2026, 10, 4, 14, 7, 0)
            .unwrap()
            .timestamp_millis()
    }
    fn at(h: u32, m: u32, s: u32) -> i64 {
        Utc.with_ymd_and_hms(2026, 10, 4, h, m, s)
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

    fn one_sector() -> SectorTable {
        SectorTable {
            volumes: vec![volume("ZDC", "01001")],
        }
    }

    /// The first bin of the only sector.
    fn first_bin(table: &SectorTable, tracks: &[Track]) -> BinPeak {
        sector_loads(table, tracks, now())[0].bins[0]
    }

    /// AC2: four flights each inside for a few minutes, one after another — the peak is 1, not 4.
    #[test]
    fn flights_crossing_in_sequence_peak_at_one_not_four() {
        let fixes: Vec<Vec<Fix>> = (0..4)
            .map(|i| (0..3).map(|m| inside(at(14, i * 3 + m, 30))).collect())
            .collect();
        let tracks: Vec<Track> = ["A", "B", "C", "D"]
            .iter()
            .zip(&fixes)
            .map(|(id, f)| Track {
                id,
                population: Population::Active,
                fixes: f,
            })
            .collect();
        assert_eq!(first_bin(&one_sector(), &tracks).active, 1);
    }

    /// AC3: in, out and back in within one minute is one flight in that minute.
    #[test]
    fn a_boundary_skim_within_a_minute_counts_once() {
        let fixes = [
            inside(at(14, 1, 10)),
            outside(at(14, 1, 30)),
            inside(at(14, 1, 50)),
        ];
        let track = Track {
            id: "SKIM",
            population: Population::Active,
            fixes: &fixes,
        };
        assert_eq!(first_bin(&one_sector(), &[track]).active, 1);
    }

    /// AC4: the active peak (2, minute 0) and proposed peak (3, minute 5) fall in different minutes.
    /// The combined peak is the busiest single minute (4), not the sum of the peaks (5).
    #[test]
    fn the_combined_peak_is_maxed_minute_by_minute() {
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

    /// AC5: bins are absolute quarter-hours — at 1407Z the first is 1400 — over six hours.
    #[test]
    fn bins_align_to_the_zulu_quarter_hour() {
        let fixes = [
            inside(at(13, 59, 0)),  // before the first bin: ignored
            inside(at(14, 14, 59)), // last minute of bin 0
            inside(at(14, 15, 0)),  // first minute of bin 1
            inside(at(20, 0, 0)),   // six hours after 1400: past the horizon
        ];
        let track = Track {
            id: "A",
            population: Population::Active,
            fixes: &fixes,
        };
        let row = &sector_loads(&one_sector(), &[track], now())[0];
        assert_eq!(row.bins.len(), 24);
        assert_eq!(row.bins[0].start_ms, at(14, 0, 0));
        assert_eq!(row.bins[1].start_ms, at(14, 15, 0));
        let counts: Vec<usize> = row.bins.iter().map(|b| b.active).collect();
        assert_eq!(&counts[..2], [1, 1]);
        assert_eq!(counts[2..].iter().sum::<usize>(), 0);
    }

    /// A sector stored as two volumes is one row, and a flight in both pieces in one minute is one.
    #[test]
    fn a_sector_split_into_pieces_counts_a_flight_once() {
        let mut second = volume("ZDC", "01002");
        second.sector_id = "010".into(); // same sector as `volume("ZDC", "01001")`
        let table = SectorTable {
            volumes: vec![volume("ZDC", "01001"), second],
        };
        let fixes = [inside(at(14, 2, 0))];
        let track = Track {
            id: "A",
            population: Population::Active,
            fixes: &fixes,
        };
        let loads = sector_loads(&table, &[track], now());
        assert_eq!(loads.len(), 1);
        assert_eq!(loads[0].bins[0].active, 1);
    }

    /// Containment is #596's: below the floor is not in the sector. A sector with no traffic still
    /// gets its row of zeros.
    #[test]
    fn altitude_counts_and_an_empty_sector_still_has_a_row() {
        let mut high = volume("ZDC", "02001");
        high.base_alt_ft = 24_000;
        high.top_alt_ft = 60_000;
        let table = SectorTable {
            volumes: vec![high],
        };
        let fixes = [inside(at(14, 3, 0))]; // at 10,000 ft
        let track = Track {
            id: "LOW",
            population: Population::Active,
            fixes: &fixes,
        };
        let loads = sector_loads(&table, &[track], now());
        assert_eq!(loads.len(), 1);
        assert!(loads[0].bins.iter().all(|b| b.combined == 0));
    }
}
