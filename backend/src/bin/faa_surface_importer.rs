//! Offline importer: bundles US airport surface geometry (ramp/apron areas, taxiways) from the
//! FAA's Aeronautical Information Services "Aerodrome Mapping" (AM) ArcGIS feature services into
//! a committed data file.
//!
//! Run by hand, occasionally, to refresh `backend/data/faa_surface.json` — never called at
//! runtime or from CI. This supersedes the OSM/Overpass-based importer from #254: Overpass proved
//! too unreliable to run nationwide, and OSM's ODbL license clashes with this project's MIT
//! license. The FAA AM layers are a U.S. Government work (public domain, MIT-clean), surveyed to
//! AC 150/5300-18, and served by a robust, paginated, first-party ArcGIS REST API — no per-airport
//! bounding-box queries or rate limiting needed, unlike Overpass. See VATUSA/OIS#230.
//!
//! ```text
//! cargo run -p ois-backend --bin faa-surface-importer
//! ```
//! No database connection needed — this only makes HTTP calls to the FAA's ArcGIS services and
//! writes the output file directly. A full run pages through ~20k taxiway and ~4k apron features
//! nationwide, which completes in well under a minute against this first-party service.
//!
//! ## Output schema
//!
//! `backend/data/faa_surface.json` is a JSON object keyed by uppercase ICAO. There is no `gates`
//! key: the FAA AM layer set has taxiway/apron/runway layers but **no parking-stand/gate layer**
//! (confirmed: neither AM nor CIFP carries gate records), so gates remain sourced solely from the
//! permissioned manual editor (#178/#190). Field names below match the corresponding
//! `flow.airport_taxiway` / `flow.airport_ramp_area` columns 1:1, so #231's upsert is a direct
//! field copy:
//!
//! ```json
//! {
//!   "KDCA": {
//!     "taxiways": [{ "name": "M", "points": [[38.85, -77.04], ...] }],
//!     "ramps": [{ "name": "GENERAL AVIATION PARKING", "kind": "apron", "rings": [[[38.85, -77.04], ...]] }]
//!   }
//! }
//! ```
//! - `taxiways`: one entry per `AM_Taxiway` polygon feature. **This is pavement (a polygon), not a
//!   centerline** — FAA models taxiways as filled shapes, unlike OSM's line geometry. `points`
//!   holds the polygon's exterior ring verbatim as `[lat, lon]` pairs (a closed ring, not a line);
//!   any interior rings (holes) are dropped — a live nationwide sample of 500 features found none,
//!   so this is expected to be a no-op in practice, not a real data loss.
//! - `ramps`: one entry per `AM_Apron` polygon feature; `kind` is always `"apron"`, matching this
//!   codebase's existing `flow.airport_ramp_area.kind` convention (the `"ramp"` kind stays
//!   reserved for manual/CRC-imported rows). `rings` holds a single ring (exterior only, same
//!   simplification as taxiways).
//! - `name` is the feature's `DESIGNATOR` field (e.g. taxiway letter, apron use description), or
//!   `FAA-<OBJECTID>` when `DESIGNATOR` is blank.
//! - Features with no `ICAO_ID` are skipped (not resolved via an FAA_ID→ICAO crosswalk) — measured
//!   live at under 0.5% of records nationwide (79/20,103 taxiways, 13/4,096 aprons), not worth a
//!   new bundled crosswalk dataset for.
//! - A feature whose geometry isn't `Polygon` is also skipped — in practice this is the rare
//!   `MultiPolygon` (measured live: 1 of 24,199 features nationwide, one apron with several
//!   disjoint parking areas grouped as one record).

use std::{collections::HashMap, time::Duration};

use serde::{Deserialize, Serialize};

const TAXIWAY_URL: &str =
    "https://services6.arcgis.com/ssFJjBXIUyZDrSYZ/arcgis/rest/services/AM_Taxiway/FeatureServer/0";
const APRON_URL: &str =
    "https://services6.arcgis.com/ssFJjBXIUyZDrSYZ/arcgis/rest/services/AM_Apron/FeatureServer/0";
const PAGE_SIZE: usize = 2000;
/// Safety cap on pages per layer (nationwide totals are ~11 pages at `PAGE_SIZE`) — guards
/// against an infinite loop if the service ever ignores `resultOffset`.
const MAX_PAGES: usize = 50;
const MAX_ATTEMPTS: u32 = 4;
/// Delay between page requests — the org's ArcGIS quota is request-unit based (cost scales with
/// records returned, not just request count), so a tight loop over full 2000-record pages can
/// trip it well before hitting `MAX_PAGES`. Observed live: ~6000 units/minute.
const PAGE_DELAY: Duration = Duration::from_millis(1500);

#[derive(Debug, Default, Serialize)]
struct AirportSurface {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    taxiways: Vec<TaxiwayRow>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    ramps: Vec<RampRow>,
}

#[derive(Debug, Serialize)]
struct TaxiwayRow {
    name: String,
    points: Vec<[f64; 2]>,
}

#[derive(Debug, Serialize)]
struct RampRow {
    name: String,
    kind: &'static str,
    rings: Vec<Vec<[f64; 2]>>,
}

#[derive(Debug, Default, Deserialize)]
struct FeatureCollection {
    #[serde(default)]
    features: Vec<Feature>,
    /// ArcGIS reports quota/query errors as an `error` object in an otherwise-200 response, not
    /// as an HTTP error status — an unchecked response looks like a valid, empty page.
    error: Option<ArcgisError>,
}

#[derive(Debug, Deserialize)]
struct ArcgisError {
    code: i32,
    message: String,
}

#[derive(Debug, Deserialize)]
struct Feature {
    geometry: Geometry,
    properties: Properties,
}

#[derive(Debug, Default, Deserialize)]
struct Geometry {
    #[serde(rename = "type", default)]
    kind: String,
    /// Kept as raw JSON rather than a typed shape: `Polygon` coordinates are rings of positions,
    /// but the rare `MultiPolygon` feature nests one level deeper (a list of polygons) and would
    /// fail to deserialize into a fixed `Polygon` shape. Only `Polygon` is mapped (see
    /// `exterior_ring_lat_lon`) — nationwide, exactly 1 of 24,199 AM_Taxiway/AM_Apron features is
    /// a `MultiPolygon`; skipping it outright is simpler than parsing two coordinate shapes for
    /// one record's benefit.
    #[serde(default)]
    coordinates: serde_json::Value,
}

#[derive(Debug, Default, Deserialize)]
struct Properties {
    #[serde(rename = "ICAO_ID")]
    icao_id: Option<String>,
    #[serde(rename = "DESIGNATOR")]
    designator: Option<String>,
    #[serde(rename = "OBJECTID")]
    object_id: Option<i64>,
}

/// Counts of what happened while mapping one layer's features, for the end-of-run summary log.
#[derive(Debug, Default)]
struct MapStats {
    skipped_no_icao: usize,
    multi_ring_seen: usize,
    skipped_no_geometry: usize,
}

/// Rounds to 7 decimal degrees (~1cm) — the source ArcGIS coordinates carry 10+ digits of
/// float noise far beyond the FAA's own ~3ft (AC 150/5300-18) survey accuracy, and left
/// unrounded that noise alone roughly triples the committed extract's file size.
fn round_deg(v: f64) -> f64 {
    (v * 1e7).round() / 1e7
}

/// The exterior ring of a `Polygon` geometry as `[lat, lon]` pairs. Any other geometry type
/// (in practice, the rare `MultiPolygon`) returns `None` — see the `Geometry::coordinates` doc.
fn exterior_ring_lat_lon(geometry: &Geometry) -> Option<Vec<[f64; 2]>> {
    if geometry.kind != "Polygon" {
        return None;
    }
    let ring = geometry.coordinates.as_array()?.first()?.as_array()?;
    Some(
        ring.iter()
            .filter_map(|p| {
                let p = p.as_array()?;
                Some([
                    round_deg(p.get(1)?.as_f64()?),
                    round_deg(p.first()?.as_f64()?),
                ])
            })
            .collect(),
    )
}

/// Number of rings in a `Polygon` geometry (1 = no holes); 0 for any other geometry type.
fn ring_count(geometry: &Geometry) -> usize {
    if geometry.kind != "Polygon" {
        return 0;
    }
    geometry
        .coordinates
        .as_array()
        .map(Vec::len)
        .unwrap_or_default()
}

fn feature_name(designator: Option<&str>, object_id: Option<i64>) -> String {
    designator
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("FAA-{}", object_id.unwrap_or_default()))
}

fn map_taxiways(features: &[Feature]) -> (HashMap<String, Vec<TaxiwayRow>>, MapStats) {
    let mut by_icao: HashMap<String, Vec<TaxiwayRow>> = HashMap::new();
    let mut stats = MapStats::default();

    for f in features {
        let Some(icao) = f
            .properties
            .icao_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_ascii_uppercase)
        else {
            stats.skipped_no_icao += 1;
            continue;
        };
        if ring_count(&f.geometry) > 1 {
            stats.multi_ring_seen += 1;
        }
        let Some(points) = exterior_ring_lat_lon(&f.geometry) else {
            stats.skipped_no_geometry += 1;
            continue;
        };
        by_icao.entry(icao).or_default().push(TaxiwayRow {
            name: feature_name(f.properties.designator.as_deref(), f.properties.object_id),
            points,
        });
    }

    (by_icao, stats)
}

fn map_ramps(features: &[Feature]) -> (HashMap<String, Vec<RampRow>>, MapStats) {
    let mut by_icao: HashMap<String, Vec<RampRow>> = HashMap::new();
    let mut stats = MapStats::default();

    for f in features {
        let Some(icao) = f
            .properties
            .icao_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_ascii_uppercase)
        else {
            stats.skipped_no_icao += 1;
            continue;
        };
        if ring_count(&f.geometry) > 1 {
            stats.multi_ring_seen += 1;
        }
        let Some(ring) = exterior_ring_lat_lon(&f.geometry) else {
            stats.skipped_no_geometry += 1;
            continue;
        };
        by_icao.entry(icao).or_default().push(RampRow {
            name: feature_name(f.properties.designator.as_deref(), f.properties.object_id),
            kind: "apron",
            rings: vec![ring],
        });
    }

    (by_icao, stats)
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("ois-faa-surface-importer/0.1 (+https://vatusa.net)")
        .timeout(Duration::from_secs(30))
        .build()
        .expect("failed to build FAA AM HTTP client")
}

async fn fetch_page(
    http: &reqwest::Client,
    base_url: &str,
    offset: usize,
) -> Result<FeatureCollection, String> {
    let url = format!(
        "{base_url}/query?where=1%3D1&outFields=ICAO_ID,DESIGNATOR,OBJECTID&f=geojson&resultRecordCount={PAGE_SIZE}&resultOffset={offset}"
    );
    let mut last_err = String::new();

    for attempt in 1..=MAX_ATTEMPTS {
        let mut backoff = Duration::from_secs(2 * attempt as u64);
        match http.get(&url).send().await {
            Ok(r) if r.status().is_success() => match r.json::<FeatureCollection>().await {
                Ok(page) => match page.error {
                    // ArcGIS reports its own quota/query errors as a 200 with an `error` body —
                    // its 429 carries a "Retry after 60 sec" quota window, so back off longer
                    // than the default HTTP-error backoff below.
                    Some(err) if err.code == 429 => {
                        last_err = format!(
                            "{base_url} offset {offset}: quota error (attempt {attempt}): {}",
                            err.message
                        );
                        backoff = Duration::from_secs(65);
                    }
                    Some(err) => {
                        return Err(format!(
                            "{base_url} offset {offset}: ArcGIS error {}: {}",
                            err.code, err.message
                        ));
                    }
                    None => return Ok(page),
                },
                Err(e) => last_err = format!("bad JSON at offset {offset}: {e}"),
            },
            Ok(r) => {
                last_err = format!("HTTP {} at offset {offset} (attempt {attempt})", r.status());
            }
            Err(e) => {
                last_err = format!("{e} at offset {offset} (attempt {attempt})");
            }
        }
        tokio::time::sleep(backoff).await;
    }
    Err(last_err)
}

/// Pages through a layer's full feature set. Stops on the first empty page rather than comparing
/// against `PAGE_SIZE`, so it adapts automatically if the service enforces a smaller
/// `maxRecordCount` than requested.
async fn fetch_all_features(
    http: &reqwest::Client,
    base_url: &str,
) -> Result<Vec<Feature>, String> {
    let mut all = Vec::new();
    let mut offset = 0usize;

    for _ in 0..MAX_PAGES {
        let page = fetch_page(http, base_url, offset).await?;
        let n = page.features.len();
        if n == 0 {
            return Ok(all);
        }
        offset += n;
        all.extend(page.features);
        tokio::time::sleep(PAGE_DELAY).await;
    }
    Err(format!(
        "{base_url}: exceeded {MAX_PAGES} pages without an empty page — resultOffset may not be advancing"
    ))
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let http = client();

    tracing::info!("fetching AM_Taxiway features");
    let taxiway_features = fetch_all_features(&http, TAXIWAY_URL)
        .await
        .expect("failed to fetch AM_Taxiway features");
    let (taxiways_by_icao, taxiway_stats) = map_taxiways(&taxiway_features);

    tracing::info!("fetching AM_Apron features");
    let apron_features = fetch_all_features(&http, APRON_URL)
        .await
        .expect("failed to fetch AM_Apron features");
    let (ramps_by_icao, apron_stats) = map_ramps(&apron_features);

    let mut extract: HashMap<String, AirportSurface> = HashMap::new();
    for (icao, taxiways) in taxiways_by_icao {
        extract.entry(icao).or_default().taxiways = taxiways;
    }
    for (icao, ramps) in ramps_by_icao {
        extract.entry(icao).or_default().ramps = ramps;
    }

    tracing::info!(
        airports = extract.len(),
        taxiway_features = taxiway_features.len(),
        apron_features = apron_features.len(),
        taxiway_skipped_no_icao = taxiway_stats.skipped_no_icao,
        apron_skipped_no_icao = apron_stats.skipped_no_icao,
        taxiway_multi_ring_seen = taxiway_stats.multi_ring_seen,
        apron_multi_ring_seen = apron_stats.multi_ring_seen,
        "done"
    );

    let out_path = concat!(env!("CARGO_MANIFEST_DIR"), "/data/faa_surface.json");
    // Compact, not pretty — this is a large array-of-coordinates file; per-element indentation
    // would multiply its size for a file nobody reads by hand.
    let out = serde_json::to_string(&extract).expect("extract should serialize");
    std::fs::write(out_path, out).expect("failed to write output file");
    tracing::info!(path = out_path, count = extract.len(), "wrote output file");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feature(
        icao: Option<&str>,
        designator: Option<&str>,
        object_id: i64,
        coords: serde_json::Value,
    ) -> Feature {
        serde_json::from_value(serde_json::json!({
            "geometry": { "type": "Polygon", "coordinates": coords },
            "properties": {
                "ICAO_ID": icao,
                "DESIGNATOR": designator,
                "OBJECTID": object_id,
            }
        }))
        .unwrap()
    }

    #[test]
    fn maps_a_realistic_taxiway_page() {
        let features = vec![
            feature(
                Some("kdca"),
                Some("M"),
                177,
                serde_json::json!([[[-77.04, 38.85], [-77.041, 38.851], [-77.04, 38.85]]]),
            ),
            feature(None, Some("ORPHAN"), 999, serde_json::json!([[[0.0, 0.0]]])),
            feature(
                Some("KMEM"),
                None,
                42,
                serde_json::json!([[[-89.97, 35.03]]]),
            ),
        ];

        let (by_icao, stats) = map_taxiways(&features);

        assert_eq!(stats.skipped_no_icao, 1);
        assert_eq!(stats.multi_ring_seen, 0);

        let kdca = &by_icao["KDCA"][0];
        assert_eq!(kdca.name, "M");
        assert_eq!(
            kdca.points,
            vec![[38.85, -77.04], [38.851, -77.041], [38.85, -77.04]]
        );

        // blank DESIGNATOR falls back to a stable FAA-<OBJECTID> name.
        assert_eq!(by_icao["KMEM"][0].name, "FAA-42");
    }

    #[test]
    fn maps_aprons_with_kind_always_apron() {
        let features = vec![feature(
            Some("KDCA"),
            Some("GENERAL AVIATION PARKING"),
            1255,
            serde_json::json!([[[-77.044, 38.846], [-77.043, 38.845], [-77.044, 38.846]]]),
        )];

        let (by_icao, stats) = map_ramps(&features);

        assert_eq!(stats.skipped_no_icao, 0);
        let ramp = &by_icao["KDCA"][0];
        assert_eq!(ramp.kind, "apron");
        assert_eq!(ramp.rings.len(), 1);
        assert_eq!(ramp.name, "GENERAL AVIATION PARKING");
    }

    #[test]
    fn a_hole_in_the_polygon_is_dropped_but_counted() {
        let features = vec![feature(
            Some("KDCA"),
            Some("A"),
            1,
            serde_json::json!([
                [[-77.04, 38.85], [-77.041, 38.851], [-77.04, 38.85]],
                [[-77.0395, 38.8505]],
            ]),
        )];

        let (by_icao, stats) = map_taxiways(&features);

        assert_eq!(stats.multi_ring_seen, 1);
        // only the exterior ring's points are kept.
        assert_eq!(by_icao["KDCA"][0].points.len(), 3);
    }

    #[test]
    fn a_200_response_carrying_an_error_body_is_not_treated_as_an_empty_page() {
        // ArcGIS reports quota/query errors as a 200 with an `error` object, not an HTTP error
        // status — this must deserialize with `error` populated, not silently as zero features.
        let page: FeatureCollection = serde_json::from_value(serde_json::json!({
            "error": {
                "code": 429,
                "message": "Unable to perform query. Too many requests."
            }
        }))
        .unwrap();
        assert!(page.features.is_empty());
        let err = page.error.expect("error body should be captured");
        assert_eq!(err.code, 429);
    }

    #[test]
    fn coordinates_are_rounded_to_1cm_precision() {
        let features = vec![feature(
            Some("KDCA"),
            Some("M"),
            1,
            serde_json::json!([[[-77.0416274123456, 38.8520919987654]]]),
        )];
        let (by_icao, _) = map_taxiways(&features);
        assert_eq!(by_icao["KDCA"][0].points, vec![[38.852092, -77.0416274]]);
    }

    #[test]
    fn empty_page_yields_no_rows() {
        let (by_icao, stats) = map_taxiways(&[]);
        assert!(by_icao.is_empty());
        assert_eq!(stats.skipped_no_icao, 0);
    }

    #[test]
    fn a_feature_with_no_rings_is_skipped_and_counted() {
        let features = vec![feature(Some("KDCA"), Some("X"), 1, serde_json::json!([]))];
        let (by_icao, stats) = map_taxiways(&features);
        assert!(by_icao.is_empty());
        assert_eq!(stats.skipped_no_geometry, 1);
    }

    #[test]
    fn a_multipolygon_feature_is_skipped_and_counted_not_misparsed() {
        // Real example seen live in AM_Apron: a MultiPolygon nests one level deeper than
        // Polygon's ring-of-positions shape. Must be skipped cleanly, not crash the whole page.
        let multipolygon: Feature = serde_json::from_value(serde_json::json!({
            "geometry": {
                "type": "MultiPolygon",
                "coordinates": [[[[-77.04, 38.85], [-77.041, 38.851], [-77.04, 38.85]]]]
            },
            "properties": { "ICAO_ID": "KDCA", "DESIGNATOR": "X", "OBJECTID": 1 }
        }))
        .unwrap();

        let (by_icao, stats) = map_ramps(&[multipolygon]);
        assert!(by_icao.is_empty());
        assert_eq!(stats.skipped_no_geometry, 1);
    }
}
