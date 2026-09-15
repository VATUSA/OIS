//! Offline importer: bundles US airport surface geometry (gates/parking positions, ramp/apron
//! areas, taxiways) from OpenStreetMap's Overpass API into a committed data file.
//!
//! Run by hand, occasionally, to refresh `backend/data/osm_surface.json` — never called at
//! runtime or from CI (Overpass is rate-limited and flaky; see VATUSA/OIS#230). Sub-issue B
//! (#231) upserts this file's contents into `flow.airport_gate` / `flow.airport_ramp_area` /
//! `flow.airport_taxiway`, tagging every row `source = 'osm'`.
//!
//! ```text
//! cargo run -p ois-backend --bin osm-surface-importer
//! ```
//! No database connection is needed — this only makes HTTP calls (mwgg/Airports, then Overpass)
//! and writes the output file directly, wherever the workspace checkout lives. A full run queries
//! every US airport in `runways.json` (~2,400) at roughly 1 request/second, so it takes over an
//! hour; set `OSM_IMPORTER_LIMIT=5` to smoke-test against a handful of airports first. The
//! output file is checkpointed every 200 airports, so an interrupted run (the public Overpass
//! instance can start refusing connections under sustained load — observed in practice) still
//! leaves a usable partial extract on disk instead of nothing. A re-run always starts from the
//! full airport list again (no incremental resume); that's an acceptable simplification since
//! this is meant to be run occasionally, not continuously.
//!
//! ## Output schema
//!
//! `backend/data/osm_surface.json` is a JSON object keyed by uppercase ICAO. Airports with no
//! matching OSM data are omitted entirely — OSM's `aeroway` coverage is rich at major airports
//! and sparse-to-absent at small fields, which is expected and fine (everything here is a
//! starting layer; facilities can hand-edit or re-pull per field). Field names below are chosen
//! to match the corresponding `flow.airport_*` columns 1:1, so sub-issue B's upsert is a direct
//! field copy, not a re-derivation:
//!
//! ```json
//! {
//!   "KDCA": {
//!     "gates": [{ "name": "B15", "lat": 38.8520919, "lon": -77.0416274 }],
//!     "ramps": [{ "name": "Cargo Ramp North", "kind": "apron", "rings": [[[38.85, -77.04], ...]] }],
//!     "taxiways": [{ "name": "A", "points": [[38.85, -77.04], ...] }]
//!   }
//! }
//! ```
//! - `gates`: one entry per OSM `aeroway=gate` or `aeroway=parking_position` **node** (name from
//!   its `ref` tag, then `name`, then `OSM-<id>` if neither is present).
//! - `ramps`: one entry per OSM `aeroway=apron` **way**; `kind` is always `"apron"` — OSM has no
//!   equivalent of the `"ramp"` kind, which stays reserved for manual/CRC-imported rows. `rings`
//!   holds a single ring (the way's own geometry); OSM multipolygon aprons (relations) are rare
//!   and are skipped (logged), not assembled into multi-ring polygons.
//! - `taxiways`: one entry per OSM `aeroway=taxiway` **way**.

use std::{collections::HashMap, time::Duration};

use serde::{Deserialize, Serialize};

const OVERPASS_URL: &str = "https://overpass-api.de/api/interpreter";
/// Half-width of the bounding box queried around each airport's reference point. Generous enough
/// to cover a typical single-terminal airport's gates/aprons with margin; a handful of the
/// largest hubs (DFW, DEN, ...) may only get partial coverage — acceptable per VATUSA/OIS#230
/// ("coverage is uneven ... that's acceptable, this is a starting layer").
const HALF_WIDTH_KM: f64 = 4.0;
/// Delay between Overpass requests — a public, shared, rate-limited instance; be polite.
const REQUEST_DELAY: Duration = Duration::from_millis(1100);
const MAX_ATTEMPTS: u32 = 4;

#[derive(Debug, Default, Serialize)]
struct AirportSurface {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    gates: Vec<GateRow>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    ramps: Vec<RampRow>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    taxiways: Vec<TaxiwayRow>,
}

impl AirportSurface {
    fn is_empty(&self) -> bool {
        self.gates.is_empty() && self.ramps.is_empty() && self.taxiways.is_empty()
    }
}

#[derive(Debug, Serialize)]
struct GateRow {
    name: String,
    lat: f64,
    lon: f64,
}

#[derive(Debug, Serialize)]
struct RampRow {
    name: String,
    kind: &'static str,
    rings: Vec<Vec<[f64; 2]>>,
}

#[derive(Debug, Serialize)]
struct TaxiwayRow {
    name: String,
    points: Vec<[f64; 2]>,
}

/// A single element from an Overpass `out geom;` response.
#[derive(Debug, Deserialize)]
struct Element {
    #[serde(rename = "type")]
    kind: String,
    id: u64,
    #[serde(default)]
    lat: Option<f64>,
    #[serde(default)]
    lon: Option<f64>,
    #[serde(default)]
    geometry: Vec<GeomPoint>,
    #[serde(default)]
    tags: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct GeomPoint {
    lat: f64,
    lon: f64,
}

#[derive(Debug, Deserialize)]
struct OverpassResponse {
    #[serde(default)]
    elements: Vec<Element>,
}

/// South/west/north/east bounds of a box `half_width_km` (each direction) around `(lat, lon)`.
fn bbox(lat: f64, lon: f64, half_width_km: f64) -> (f64, f64, f64, f64) {
    let lat_delta = half_width_km / 111.32;
    let lon_delta = half_width_km / (111.32 * lat.to_radians().cos());
    (
        lat - lat_delta,
        lon - lon_delta,
        lat + lat_delta,
        lon + lon_delta,
    )
}

fn overpass_query(bbox: (f64, f64, f64, f64)) -> String {
    let (south, west, north, east) = bbox;
    format!(
        "[out:json][timeout:25];\
         (nwr[\"aeroway\"~\"^(gate|parking_position|apron|taxiway)$\"]({south},{west},{north},{east}););\
         out geom;"
    )
}

fn element_name(tags: &HashMap<String, String>, id: u64) -> String {
    tags.get("ref")
        .or_else(|| tags.get("name"))
        .cloned()
        .unwrap_or_else(|| format!("OSM-{id}"))
}

/// Maps one airport's Overpass elements into its surface rows. Returns the row counts plus how
/// many elements were skipped (relations, or a way/node shape that doesn't match its `aeroway`
/// value — e.g. a gate tagged on a way instead of a node).
fn map_elements(elements: &[Element]) -> (AirportSurface, usize) {
    let mut surface = AirportSurface::default();
    let mut skipped = 0;

    for el in elements {
        let Some(aeroway) = el.tags.get("aeroway").map(String::as_str) else {
            skipped += 1;
            continue;
        };
        match (aeroway, el.kind.as_str()) {
            ("gate" | "parking_position", "node") => {
                if let (Some(lat), Some(lon)) = (el.lat, el.lon) {
                    surface.gates.push(GateRow {
                        name: element_name(&el.tags, el.id),
                        lat,
                        lon,
                    });
                } else {
                    skipped += 1;
                }
            }
            ("apron", "way") if !el.geometry.is_empty() => {
                let ring = el.geometry.iter().map(|p| [p.lat, p.lon]).collect();
                surface.ramps.push(RampRow {
                    name: element_name(&el.tags, el.id),
                    kind: "apron",
                    rings: vec![ring],
                });
            }
            ("taxiway", "way") if !el.geometry.is_empty() => {
                let points = el.geometry.iter().map(|p| [p.lat, p.lon]).collect();
                surface.taxiways.push(TaxiwayRow {
                    name: element_name(&el.tags, el.id),
                    points,
                });
            }
            _ => skipped += 1,
        }
    }

    (surface, skipped)
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("ois-osm-surface-importer/0.1 (+https://vatusa.net)")
        .timeout(Duration::from_secs(30))
        .build()
        .expect("failed to build Overpass HTTP client")
}

async fn fetch_overpass(
    http: &reqwest::Client,
    icao: &str,
    bbox: (f64, f64, f64, f64),
) -> Result<OverpassResponse, String> {
    let query = overpass_query(bbox);
    let mut last_err = String::new();

    for attempt in 1..=MAX_ATTEMPTS {
        let resp = http.post(OVERPASS_URL).body(query.clone()).send().await;
        match resp {
            Ok(r) if r.status().is_success() => {
                return r.json().await.map_err(|e| format!("{icao}: bad JSON: {e}"));
            }
            Ok(r) if r.status().as_u16() == 429 || r.status().as_u16() == 504 => {
                last_err = format!("{icao}: HTTP {} (attempt {attempt})", r.status());
                tokio::time::sleep(Duration::from_secs(5 * attempt as u64)).await;
            }
            Ok(r) => return Err(format!("{icao}: HTTP {}", r.status())),
            Err(e) => {
                last_err = format!("{icao}: {e} (attempt {attempt})");
                tokio::time::sleep(Duration::from_secs(5 * attempt as u64)).await;
            }
        }
    }
    Err(last_err)
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let http = client();

    tracing::info!("fetching US airport list from backend/data/runways.json");
    let runway_db: HashMap<String, serde_json::Value> =
        serde_json::from_str(include_str!("../../data/runways.json"))
            .expect("backend/data/runways.json should parse");
    let mut icaos: Vec<String> = runway_db.into_keys().collect();
    icaos.sort();
    // A full run queries ~2,400 airports at ~1 req/s (over an hour) against a shared public
    // instance — set this to smoke-test the pipeline against a handful of airports first.
    if let Ok(limit) = std::env::var("OSM_IMPORTER_LIMIT") {
        let limit: usize = limit
            .parse()
            .expect("OSM_IMPORTER_LIMIT should be a number");
        icaos.truncate(limit);
    }

    tracing::info!(
        count = icaos.len(),
        "fetching airport coordinates from mwgg/Airports"
    );
    let (airport_db, _iata) = ois_backend::feed::airports::fetch(&http)
        .await
        .expect("failed to fetch airport coordinates");

    let mut extract: HashMap<String, AirportSurface> = HashMap::new();
    let mut skipped_no_coords = 0usize;
    let mut total_skipped_elements = 0usize;
    let mut failed: Vec<String> = Vec::new();

    for (i, icao) in icaos.iter().enumerate() {
        let Some(&(lat, lon)) = airport_db.get(icao) else {
            skipped_no_coords += 1;
            continue;
        };

        match fetch_overpass(&http, icao, bbox(lat, lon, HALF_WIDTH_KM)).await {
            Ok(resp) => {
                let (surface, skipped) = map_elements(&resp.elements);
                total_skipped_elements += skipped;
                if !surface.is_empty() {
                    extract.insert(icao.clone(), surface);
                }
            }
            Err(e) => {
                tracing::warn!("{e}");
                failed.push(icao.clone());
            }
        }

        if (i + 1) % 100 == 0 {
            tracing::info!(
                done = i + 1,
                total = icaos.len(),
                found = extract.len(),
                "progress"
            );
        }
        // A full run takes over an hour against a shared public instance that can start
        // refusing connections partway through (observed running this for real) — checkpoint
        // periodically so an interrupted run still leaves a usable, re-runnable partial extract
        // instead of nothing.
        if (i + 1) % 200 == 0 {
            write_extract(&extract);
        }
        tokio::time::sleep(REQUEST_DELAY).await;
    }

    tracing::info!(
        airports_with_data = extract.len(),
        airports_queried = icaos.len(),
        skipped_no_coords,
        skipped_elements = total_skipped_elements,
        failed = failed.len(),
        "done"
    );
    if !failed.is_empty() {
        tracing::warn!(
            ?failed,
            "these airports failed after retries and were skipped"
        );
    }

    write_extract(&extract);
}

fn write_extract(extract: &HashMap<String, AirportSurface>) {
    let out_path = concat!(env!("CARGO_MANIFEST_DIR"), "/data/osm_surface.json");
    let out = serde_json::to_string_pretty(extract).expect("extract should serialize");
    std::fs::write(out_path, out).expect("failed to write output file");
    tracing::info!(path = out_path, count = extract.len(), "wrote output file");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bbox_widens_at_higher_latitude() {
        let (s, w, n, e) = bbox(38.85, -77.04, 4.0);
        assert!(s < 38.85 && n > 38.85);
        assert!(w < -77.04 && e > -77.04);
        // Longitude degrees are narrower at higher latitude, so the box should be wider in
        // degrees of longitude than of latitude for a non-equatorial airport.
        assert!((e - w) > (n - s));
    }

    #[test]
    fn maps_a_realistic_overpass_response() {
        let json = serde_json::json!({
            "elements": [
                {"type": "node", "id": 1, "lat": 38.852, "lon": -77.041, "tags": {"aeroway": "gate", "ref": "B15"}},
                {"type": "node", "id": 2, "lat": 38.853, "lon": -77.042, "tags": {"aeroway": "parking_position"}},
                {
                    "type": "way", "id": 3, "tags": {"aeroway": "apron", "name": "Cargo Ramp"},
                    "geometry": [{"lat": 38.85, "lon": -77.04}, {"lat": 38.851, "lon": -77.041}, {"lat": 38.85, "lon": -77.04}]
                },
                {
                    "type": "way", "id": 4, "tags": {"aeroway": "taxiway", "ref": "A"},
                    "geometry": [{"lat": 38.86, "lon": -77.03}, {"lat": 38.861, "lon": -77.031}]
                },
                {"type": "relation", "id": 5, "tags": {"aeroway": "apron"}},
                {"type": "node", "id": 6, "tags": {"amenity": "restaurant"}}
            ]
        });
        let resp: OverpassResponse = serde_json::from_value(json).unwrap();
        let (surface, skipped) = map_elements(&resp.elements);

        assert_eq!(surface.gates.len(), 2);
        assert_eq!(surface.gates[0].name, "B15");
        assert_eq!(surface.gates[1].name, "OSM-2");

        assert_eq!(surface.ramps.len(), 1);
        assert_eq!(surface.ramps[0].name, "Cargo Ramp");
        assert_eq!(surface.ramps[0].kind, "apron");
        assert_eq!(surface.ramps[0].rings[0].len(), 3);

        assert_eq!(surface.taxiways.len(), 1);
        assert_eq!(surface.taxiways[0].name, "A");
        assert_eq!(surface.taxiways[0].points.len(), 2);

        // relation (id 5) and the untagged node (id 6) are both skipped.
        assert_eq!(skipped, 2);
    }

    #[test]
    fn empty_response_yields_no_rows() {
        let (surface, skipped) = map_elements(&[]);
        assert!(surface.is_empty());
        assert_eq!(skipped, 0);
    }
}
