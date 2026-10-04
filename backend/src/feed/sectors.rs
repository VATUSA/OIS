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

/// Every stored volume, as cached in `AppState`.
#[derive(Debug, Clone, Default)]
pub struct SectorTable {
    pub volumes: Vec<SectorVolume>,
}

impl SectorTable {
    /// One ARTCC's sectors, each once, as `(sector_id, name)` ordered by sector — a sector spans one
    /// or more volumes, so this is what an Airspace Monitor row is.
    pub fn sectors_of(&self, artcc: &str) -> Vec<(String, Option<String>)> {
        let mut sectors = std::collections::BTreeMap::new();
        for v in self.volumes.iter().filter(|v| v.artcc == artcc) {
            let name = sectors.entry(v.sector_id.clone()).or_insert(None);
            if name.is_none() {
                *name = v.name.clone();
            }
        }
        sectors.into_iter().collect()
    }
}

/// The Monitor Alert Parameter a sector reads until a TMU overrides it (#598). Taken from vTBFM, which
/// tunes it down from real high-sector values of about 16–20 for VATSIM traffic levels.
pub const DEFAULT_MAP: i32 = 10;

/// Stored MAP overrides by `(artcc, sector_id)`, as cached in `AppState::sector_maps`.
pub type SectorMaps = std::collections::HashMap<(String, String), i32>;

/// A sector's MAP: its override, or [`DEFAULT_MAP`].
pub fn map_for(maps: &SectorMaps, artcc: &str, sector_id: &str) -> i32 {
    maps.get(&(artcc.to_string(), sector_id.to_string()))
        .copied()
        .unwrap_or(DEFAULT_MAP)
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

    #[test]
    fn a_volume_with_no_rings_is_rejected() {
        let mut v = volume("ZDC", "01001");
        v.rings.clear();
        assert!(validate_volume(&v).is_err());
    }
}
