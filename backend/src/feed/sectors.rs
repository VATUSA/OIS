//! ATC sector volumes (#594): altitude-bounded polygons OIS owns in `flow.airspace_sector`
//! (migration 0111), for the Airspace Monitor (#593).
//!
//! No public feed supplies these, so they are imported offline by `bin/airspace_sector_importer.rs`
//! and loaded into `AppState::airspace_sectors` by `jobs::spawn_airspace_sectors_refresh`. The
//! importer is a separate process, so nothing in the server can force-reload on its write: a new
//! import shows up on the job's next tick. An in-app editor would call `repos::airspace_sectors::
//! load_all` and store the result after each write, as `handlers::aircraft_profiles` does.
//!
//! Pure data and validation only — the feed reads this through the cache and never queries.

use crate::feed::airspace::point_in_ring;

/// One stored volume. A sector can be several volumes (the source splits some into pieces), so
/// `volume_id` — not `sector_id` — is unique within an ARTCC.
#[derive(Debug, Clone, PartialEq)]
pub struct SectorVolume {
    pub artcc: String,
    pub sector_id: String,
    pub volume_id: String,
    pub name: Option<String>,
    /// `low`, `high`, `ultra_high` or `approach`.
    pub tier: String,
    pub base_alt_ft: i32,
    pub top_alt_ft: i32,
    /// Closed `[lat, lon]` rings, one per polygon part (no holes).
    pub rings: Vec<Vec<[f64; 2]>>,
}

impl SectorVolume {
    /// Whether a point at `alt_ft` is inside this volume (#596): laterally inside a ring **and** in the
    /// half-open band `base_alt_ft <= alt < top_alt_ft`, so a sector's top is the next stratum's floor
    /// and stacked strata over one footprint never both claim an altitude.
    ///
    /// One altitude, supplied by the caller (the trajectory's predicted altitude at that point) — not
    /// the FCA's "filed **or** current" rule, which would count a climber below the floor and could
    /// count one aircraft in two stacked sectors. An unknown altitude (`None`) fails open: laterally
    /// inside counts.
    pub fn contains(&self, lat: f64, lon: f64, alt_ft: Option<f64>) -> bool {
        alt_ft.is_none_or(|a| f64::from(self.base_alt_ft) <= a && a < f64::from(self.top_alt_ft))
            && self.rings.iter().any(|r| point_in_ring(r, lat, lon))
    }
}

/// Every stored volume, as cached in `AppState`.
#[derive(Debug, Clone, Default)]
pub struct SectorTable {
    pub volumes: Vec<SectorVolume>,
}

impl SectorTable {
    /// Every volume containing the point — see [`SectorVolume::contains`]. Several can match: a
    /// sector may be split into pieces, and an unknown altitude matches every stratum.
    pub fn containing(
        &self,
        lat: f64,
        lon: f64,
        alt_ft: Option<f64>,
    ) -> impl Iterator<Item = &SectorVolume> {
        self.volumes
            .iter()
            .filter(move |v| v.contains(lat, lon, alt_ft))
    }
}

/// Why a volume can't be stored, or `Ok` if it can. The single gate every write passes through
/// (`repos::airspace_sectors::replace_artcc`), so nothing in the table fails it.
pub fn validate_volume(v: &SectorVolume) -> Result<(), String> {
    if v.base_alt_ft < 0 || v.base_alt_ft >= v.top_alt_ft {
        return Err(format!(
            "altitudes {}..{} ft are not a positive band",
            v.base_alt_ft, v.top_alt_ft
        ));
    }
    if v.rings.is_empty() {
        return Err("no rings".into());
    }
    v.rings.iter().try_for_each(|r| validate_ring(r))
}

/// A ring the map can draw: closed, at least a triangle, in range, and topologically simple.
/// "Simple" is what `web/src/components/map/lib/geo.ts::sanitizeRingTopology` demands — a ring that
/// revisits a vertex is a bridged ring (it tessellates into a wedge across the map, bug #481), and
/// one whose edges cross is not a polygon at all.
pub fn validate_ring(ring: &[[f64; 2]]) -> Result<(), String> {
    if ring.len() < 4 {
        return Err(format!("ring has {} points, needs at least 4", ring.len()));
    }
    if ring.first() != ring.last() {
        return Err("ring is not closed".into());
    }
    if let Some([lat, lon]) = ring
        .iter()
        .find(|[lat, lon]| !(-90.0..=90.0).contains(lat) || !(-180.0..=180.0).contains(lon))
    {
        return Err(format!("[{lat}, {lon}] is out of range"));
    }
    let open = &ring[..ring.len() - 1];
    for (i, p) in open.iter().enumerate() {
        if open[i + 1..].contains(p) {
            return Err(format!("ring revisits [{}, {}] (bridged)", p[0], p[1]));
        }
    }
    let n = open.len();
    for i in 0..n {
        for j in i + 2..n {
            // Edges i and i+1 share a vertex, as do the last and first through the closing point.
            if i == 0 && j == n - 1 {
                continue;
            }
            if segments_cross(open[i], open[(i + 1) % n], open[j], open[(j + 1) % n]) {
                return Err("ring edges cross".into());
            }
        }
    }
    Ok(())
}

/// Whether two segments properly cross (each strictly separates the other's endpoints). Planar on
/// raw lat/lon, which is fine for a topology test at sector scale.
fn segments_cross(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let side = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
        ((q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])).signum()
    };
    let (d1, d2, d3, d4) = (side(a, b, c), side(a, b, d), side(c, d, a), side(c, d, b));
    d1 * d2 < 0.0 && d3 * d4 < 0.0
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A valid one-ring volume: a small square near KDCA, 0..23,000 ft.
    pub(crate) fn volume(artcc: &str, volume_id: &str) -> SectorVolume {
        SectorVolume {
            artcc: artcc.into(),
            sector_id: volume_id[..3].into(),
            volume_id: volume_id.into(),
            name: None,
            tier: "low".into(),
            base_alt_ft: 0,
            top_alt_ft: 23_000,
            rings: vec![vec![
                [38.0, -77.0],
                [38.0, -76.0],
                [39.0, -76.0],
                [39.0, -77.0],
                [38.0, -77.0],
            ]],
        }
    }

    /// Two squares joined by an out-and-back edge — the shape #481 drew as a wedge.
    pub(crate) fn bridged_ring() -> Vec<[f64; 2]> {
        vec![
            [38.0, -77.0],
            [38.0, -76.0],
            [39.0, -76.0],
            [39.0, -77.0],
            [38.0, -77.0],
            [37.0, -77.0],
            [37.0, -78.0],
            [36.0, -78.0],
            [36.0, -77.0],
            [37.0, -77.0],
            [38.0, -77.0],
        ]
    }

    #[test]
    fn a_simple_closed_ring_is_valid() {
        assert_eq!(validate_volume(&volume("ZDC", "01001")), Ok(()));
    }

    #[test]
    fn a_bridged_ring_is_rejected() {
        let mut v = volume("ZDC", "01001");
        v.rings = vec![bridged_ring()];
        assert!(validate_volume(&v).unwrap_err().contains("bridged"));
    }

    #[test]
    fn a_bow_tie_is_rejected() {
        let ring = [
            [38.0, -77.0],
            [39.0, -76.0],
            [38.0, -76.0],
            [39.0, -77.0],
            [38.0, -77.0],
        ];
        assert_eq!(validate_ring(&ring), Err("ring edges cross".into()));
    }

    #[test]
    fn an_open_short_or_out_of_range_ring_is_rejected() {
        let open = [[38.0, -77.0], [38.0, -76.0], [39.0, -76.0], [39.0, -77.0]];
        assert_eq!(validate_ring(&open), Err("ring is not closed".into()));
        let short = [[38.0, -77.0], [38.0, -76.0], [38.0, -77.0]];
        assert!(validate_ring(&short).unwrap_err().contains("at least 4"));
        // [lon, lat] by mistake, at ZOA: -122 is not a latitude.
        let swapped = [
            [-122.0, 37.0],
            [-121.0, 37.0],
            [-121.0, 38.0],
            [-122.0, 37.0],
        ];
        assert!(
            validate_ring(&swapped)
                .unwrap_err()
                .contains("out of range")
        );
    }

    #[test]
    fn altitudes_must_be_a_positive_band() {
        let mut v = volume("ZTL", "07007");
        (v.base_alt_ft, v.top_alt_ft) = (60_000, 10_000);
        assert!(validate_volume(&v).is_err());
        (v.base_alt_ft, v.top_alt_ft) = (10_000, 10_000);
        assert!(validate_volume(&v).is_err());
        (v.base_alt_ft, v.top_alt_ft) = (-100, 10_000);
        assert!(validate_volume(&v).is_err());
        (v.base_alt_ft, v.top_alt_ft) = (9_999, 10_000);
        assert_eq!(validate_volume(&v), Ok(()));
    }

    /// The fixture square (38–39N, 76–77W) as an 18,000–60,000 ft high sector.
    fn high() -> SectorVolume {
        let mut v = volume("ZDC", "01001");
        (v.base_alt_ft, v.top_alt_ft) = (18_000, 60_000);
        v
    }

    const IN: (f64, f64) = (38.5, -76.5);

    /// #596 AC2: laterally inside but below the floor is not in the sector; above the floor is.
    #[test]
    fn an_aircraft_below_the_floor_is_not_in_the_sector() {
        let v = high();
        assert!(!v.contains(IN.0, IN.1, Some(12_000.0)));
        assert!(v.contains(IN.0, IN.1, Some(25_000.0)));
    }

    /// AC3: the band is half-open, and each edge is checked from both sides.
    #[test]
    fn the_floor_is_inside_and_the_top_is_not() {
        let v = high();
        assert!(!v.contains(IN.0, IN.1, Some(17_999.9)));
        assert!(v.contains(IN.0, IN.1, Some(18_000.0)));
        assert!(v.contains(IN.0, IN.1, Some(59_999.9)));
        assert!(!v.contains(IN.0, IN.1, Some(60_000.0)));
    }

    /// AC4: an unknown altitude fails open — but only laterally inside.
    #[test]
    fn an_unknown_altitude_counts_only_laterally_inside() {
        let v = high();
        assert!(v.contains(IN.0, IN.1, None));
        assert!(!v.contains(40.0, -76.5, None));
    }

    /// In band but outside every ring is outside: the altitude half can't satisfy it alone.
    #[test]
    fn in_band_but_outside_the_ring_is_outside() {
        assert!(!high().contains(40.0, -76.5, Some(25_000.0)));
    }

    /// AC5: two strata stacked on one footprint — an altitude belongs to exactly one, including at
    /// the shared boundary.
    #[test]
    fn stacked_sectors_resolve_to_one_stratum() {
        let mut low = volume("ZDC", "01001");
        (low.base_alt_ft, low.top_alt_ft) = (0, 24_000);
        let mut high = volume("ZDC", "02001");
        (high.base_alt_ft, high.top_alt_ft) = (24_000, 60_000);
        let table = SectorTable {
            volumes: vec![low, high],
        };
        let ids = |alt: f64| -> Vec<&str> {
            table
                .containing(IN.0, IN.1, Some(alt))
                .map(|v| v.volume_id.as_str())
                .collect()
        };
        assert_eq!(ids(23_999.0), ["01001"]);
        assert_eq!(ids(24_000.0), ["02001"]);
        assert_eq!(ids(70_000.0), Vec::<&str>::new());
    }

    /// A volume can have several parts: the importer makes one ring per `MultiPolygon` part. A point
    /// in any part is inside; one between the parts is not. A single-ring fixture can't tell `any`
    /// from `all`, so this one has two.
    #[test]
    fn a_point_in_either_part_of_a_multi_part_volume_is_inside() {
        let mut v = high();
        v.rings.push(vec![
            [40.0, -77.0],
            [40.0, -76.0],
            [41.0, -76.0],
            [41.0, -77.0],
            [40.0, -77.0],
        ]);
        for alt in [Some(25_000.0), None] {
            assert!(v.contains(38.5, -76.5, alt), "the first part, at {alt:?}");
            assert!(v.contains(40.5, -76.5, alt), "the second part, at {alt:?}");
            assert!(
                !v.contains(39.5, -76.5, alt),
                "the gap between them, at {alt:?}"
            );
        }
    }

    #[test]
    fn a_volume_with_no_rings_is_rejected() {
        let mut v = volume("ZDC", "01001");
        v.rings.clear();
        assert!(validate_volume(&v).is_err());
    }
}
