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
use crate::feed::sectors::{SectorMaps, SectorTable, map_for};

/// Sectors worked at another sector's position (#599): `(artcc, source) → target`, same ARTCC, already
/// flattened (a target is never itself a source), as cached in `AppState::sector_consolidations`.
pub type Consolidations = HashMap<(String, String), String>;

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
/// contains `now`. A sector stored as several volumes is one row, and so is a target with the sectors
/// consolidated into it (#599).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectorLoad {
    pub artcc: String,
    pub sector_id: String,
    /// The sectors worked at this one, sorted; non-empty marks a combined row (`ZLA25+`).
    pub consolidated: Vec<String>,
    /// The row's Monitor Alert Parameter — the target's own, never a sum: one controller, one limit.
    pub map: i32,
    pub bins: Vec<BinPeak>,
}

/// Peak occupancy for every row, sorted by `(artcc, sector_id)`. Bins are absolute Zulu
/// quarter-hours: at 1407Z the first starts at 1400.
///
/// A consolidated sector has no row of its own: its volumes are filed under its target **before**
/// counting, so the combined row counts distinct flights per minute across the union of the volumes
/// (#599). A flight crossing from source to target inside one minute is one flight there; adding the
/// rows' peaks would count it twice and add busiest minutes that fall at different times, so a
/// combined row can correctly read lower than the sum of its parts.
pub fn sector_loads(
    table: &SectorTable,
    consolidations: &Consolidations,
    maps: &SectorMaps,
    tracks: &[Track],
    now_ms: i64,
) -> Vec<SectorLoad> {
    let bin_ms = BIN_MIN * MINUTE_MS;
    let first_ms = now_ms - now_ms.rem_euclid(bin_ms);
    let end_ms = first_ms + HORIZON_MIN * MINUTE_MS;

    // The row each volume counts under: its sector's target if consolidated, else its sector.
    let row_key = |artcc: &str, sector: &str| -> (String, String) {
        let row = consolidations
            .get(&(artcc.to_string(), sector.to_string()))
            .map_or(sector, String::as_str);
        (artcc.to_string(), row.to_string())
    };
    let mut members: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for v in &table.volumes {
        let key = row_key(&v.artcc, &v.sector_id);
        let sources = members.entry(key.clone()).or_default();
        if v.sector_id != key.1 && !sources.contains(&v.sector_id) {
            sources.push(v.sector_id.clone());
        }
    }
    let rows: BTreeMap<&(String, String), usize> = members
        .keys()
        .enumerate()
        .map(|(i, key)| (key, i))
        .collect();
    let row_of_volume: Vec<usize> = table
        .volumes
        .iter()
        .map(|v| rows[&row_key(&v.artcc, &v.sector_id)])
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
            let inside = table
                .volumes
                .iter()
                .enumerate()
                .filter(|(_, v)| v.contains(fix.lat, fix.lon, fix.alt_ft));
            for (i, _) in inside {
                let sector = row_of_volume[i];
                let (active, proposed) = occupied.entry((sector, minute)).or_default();
                match track.population {
                    Population::Active => active.insert(track.id),
                    Population::Proposed => proposed.insert(track.id),
                };
            }
        }
    }

    members
        .iter()
        .enumerate()
        .map(|(sector, ((artcc, sector_id), sources))| SectorLoad {
            artcc: artcc.clone(),
            sector_id: sector_id.clone(),
            consolidated: {
                let mut sources = sources.clone();
                sources.sort();
                sources
            },
            map: map_for(maps, artcc, sector_id),
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
        sector_loads(
            table,
            &Default::default(),
            &Default::default(),
            tracks,
            now(),
        )[0]
        .bins[0]
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
        let row = &sector_loads(
            &one_sector(),
            &Default::default(),
            &Default::default(),
            &[track],
            now(),
        )[0];
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
        let loads = sector_loads(
            &table,
            &Default::default(),
            &Default::default(),
            &[track],
            now(),
        );
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
        let loads = sector_loads(
            &table,
            &Default::default(),
            &Default::default(),
            &[track],
            now(),
        );
        assert_eq!(loads.len(), 1);
        assert!(loads[0].bins.iter().all(|b| b.combined == 0));
    }

    /// Two adjacent sectors in ZLA: 18 west of 76.5W and 41 east of it, both 0–23,000 ft.
    fn eighteen_and_forty_one() -> SectorTable {
        let square = |sector: &str, west: f64, east: f64| SectorVolume {
            sector_id: sector.into(),
            rings: vec![vec![
                [38.0, west],
                [38.0, east],
                [39.0, east],
                [39.0, west],
                [38.0, west],
            ]],
            ..volume("ZLA", &format!("{sector}0"))
        };
        SectorTable {
            volumes: vec![square("18", -77.0, -76.5), square("41", -76.5, -76.0)],
        }
    }
    fn in_18(t_ms: i64) -> Fix {
        Fix {
            lon: -76.75,
            ..inside(t_ms)
        }
    }
    fn in_41(t_ms: i64) -> Fix {
        Fix {
            lon: -76.25,
            ..inside(t_ms)
        }
    }
    fn eighteen_at_41() -> Consolidations {
        [(("ZLA".to_string(), "18".to_string()), "41".to_string())].into()
    }
    fn active<'a>(id: &'a str, fixes: &'a [Fix]) -> Track<'a> {
        Track {
            id,
            population: Population::Active,
            fixes,
        }
    }

    /// #599 AC2: a flight crossing from 18 into 41 inside one minute is **one** aircraft in the merged
    /// row for that minute. Separately each row counts it, so summing their peaks would read 2.
    #[test]
    fn a_crossing_into_the_target_within_a_minute_counts_once() {
        let table = eighteen_and_forty_one();
        let fixes = [in_18(at(14, 1, 10)), in_41(at(14, 1, 40))];
        let tracks = [active("X", &fixes)];

        let apart = sector_loads(
            &table,
            &Default::default(),
            &Default::default(),
            &tracks,
            now(),
        );
        let sum: usize = apart.iter().map(|r| r.bins[0].active).sum();
        assert_eq!(sum, 2, "each sector alone sees the flight");

        let merged = sector_loads(
            &table,
            &eighteen_at_41(),
            &Default::default(),
            &tracks,
            now(),
        );
        assert_eq!(merged.len(), 1, "the source row disappears");
        assert_eq!(merged[0].sector_id, "41");
        assert_eq!(
            merged[0].consolidated,
            ["18"],
            "and is marked on the target"
        );
        assert_eq!(
            merged[0].bins[0].active, 1,
            "the union counts the aircraft once"
        );
    }

    /// #599 AC3: busiest minutes that fall at different times don't add up, so a combined row reads
    /// lower than the sum of its parts — correctly.
    #[test]
    fn a_combined_row_can_read_lower_than_the_sum_of_its_parts() {
        let table = eighteen_and_forty_one();
        let (p, q) = ([in_18(at(14, 1, 30))], [in_41(at(14, 5, 30))]);
        let tracks = [active("P", &p), active("Q", &q)];

        let apart = sector_loads(
            &table,
            &Default::default(),
            &Default::default(),
            &tracks,
            now(),
        );
        assert_eq!(apart.iter().map(|r| r.bins[0].active).sum::<usize>(), 2);
        let merged = sector_loads(
            &table,
            &eighteen_at_41(),
            &Default::default(),
            &tracks,
            now(),
        );
        assert_eq!(merged[0].bins[0].active, 1, "one aircraft at any minute");
    }

    /// #599 AC4: the combined row keeps the target's MAP — one controller, one workload limit — not
    /// the sum of its parts' (which would make the busiest arrangement the hardest to alert).
    #[test]
    fn a_combined_row_uses_the_targets_map() {
        let table = eighteen_and_forty_one();
        let maps: SectorMaps = [
            (("ZLA".to_string(), "18".to_string()), 30),
            (("ZLA".to_string(), "41".to_string()), 12),
        ]
        .into();

        let apart = sector_loads(&table, &Default::default(), &maps, &[], now());
        assert_eq!(apart.iter().map(|r| r.map).collect::<Vec<_>>(), [30, 12]);
        let merged = sector_loads(&table, &eighteen_at_41(), &maps, &[], now());
        assert_eq!(merged[0].map, 12);
    }
}
