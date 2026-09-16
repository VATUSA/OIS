//! Offline importer: bundles US airport surface geometry (ramp/apron areas, taxiways, runways) from the
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
//! cargo run -p ois-backend --bin faa-surface-importer                   # all layers
//! cargo run -p ois-backend --bin faa-surface-importer -- --runways-only  # only AM_Runway (#279)
//! ```
//! `--runways-only` merges each airport's `runways` into the existing extract, leaving its taxiways
//! and ramps untouched. It still fetches every layer, so airports are keyed exactly as a full run
//! would key them (see **Keying** below).
//! No database connection needed — this only makes HTTP calls to the FAA's ArcGIS services and
//! writes the output file directly. A full run pages through ~20k taxiway and ~4k apron features
//! nationwide, which completes in well under a minute against this first-party service.
//!
//! ## Output schema
//!
//! `backend/data/faa_surface.json` is a JSON object keyed by uppercase ICAO. There is no `gates`
//! key: the FAA AM layer set has taxiway/apron/runway layers but **no parking-stand/gate layer**
//! (confirmed: neither AM nor CIFP carries gate records), so gates remain sourced solely from the
//! permissioned manual editor (#178/#190).
//!
//! ```json
//! {
//!   "KDCA": {
//!     "taxiways": [{ "name": "M", "rings": [[[38.85, -77.04], ...]] }],
//!     "ramps": [{ "name": "GENERAL AVIATION PARKING", "kind": "apron", "rings": [[[38.85, -77.04], ...]] }],
//!     "runways": [{ "name": "01/19", "rings": [[[38.85, -77.04], ...]] }]
//!   }
//! }
//! ```
//! - `taxiways`: one entry per `AM_Taxiway` polygon feature. `rings` holds the pavement outline as a
//!   single closed ring of `[lat, lon]` pairs (first point == last point) — polygon semantics, a
//!   1:1 fit for `flow.airport_taxiway.rings` (#278). Interior rings (holes) are dropped — a live
//!   nationwide sample of 500 features found none, so this is expected to be a no-op in practice.
//! - `ramps`: one entry per `AM_Apron` polygon feature; `kind` is always `"apron"`, matching this
//!   codebase's existing `flow.airport_ramp_area.kind` convention (the `"ramp"` kind stays
//!   reserved for manual/CRC-imported rows). `rings` holds a single ring (exterior only, same
//!   simplification as taxiways) — polygon semantics, a 1:1 fit for `flow.airport_ramp_area.rings`.
//! - `runways`: one entry per `AM_Runway` polygon feature — the runway pavement outline, same ring
//!   shape as taxiways, a 1:1 fit for `flow.airport_runway.rings` (#279). Display geometry only:
//!   `data/runways.json` stays the Runway Balancer's heading/length source.
//! - `name` is the feature's `DESIGNATOR` field (e.g. taxiway letter, apron use description), or
//!   `FAA-<OBJECTID>` when `DESIGNATOR` is blank.
//! - **Keying.** The source `ICAO_ID` is unreliable: it's sometimes blank (all of KRUT's taxiways),
//!   sometimes the bare FAA id (`DTW`), and one airport can be split across two ICAO_IDs for the
//!   same `FAA_ID` (`ANC` → `KANC` + `PANC`, `HOM` → `KHOM` + `PAHO`). So every feature is keyed by
//!   one canonical ICAO per `FAA_ID` ([`canonical_icaos`]), validated against the bundled
//!   OurAirports airport set (`data/runways.json`); a feature with no resolvable ICAO is skipped
//!   and counted (in practice only genuinely non-ICAO fields, e.g. `40U`, `5C8`).
//! - A feature whose geometry isn't `Polygon` is also skipped — in practice this is the rare
//!   `MultiPolygon` (measured live: 1 of 24,199 features nationwide, one apron with several
//!   disjoint parking areas grouped as one record).

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    future::Future,
    time::Duration,
};

use serde::{Deserialize, Serialize};

const TAXIWAY_URL: &str =
    "https://services6.arcgis.com/ssFJjBXIUyZDrSYZ/arcgis/rest/services/AM_Taxiway/FeatureServer/0";
const APRON_URL: &str =
    "https://services6.arcgis.com/ssFJjBXIUyZDrSYZ/arcgis/rest/services/AM_Apron/FeatureServer/0";
const RUNWAY_URL: &str =
    "https://services6.arcgis.com/ssFJjBXIUyZDrSYZ/arcgis/rest/services/AM_Runway/FeatureServer/0";
/// Attributes requested per layer. AM_Runway carries its designator (`01/19`) in `RWY_ID` — its
/// `DESIGNATOR` is blank on almost every feature.
const SURFACE_FIELDS: &str = "ICAO_ID,FAA_ID,DESIGNATOR,OBJECTID";
const RUNWAY_FIELDS: &str = "ICAO_ID,FAA_ID,RWY_ID,OBJECTID";
const PAGE_SIZE: usize = 2000;
/// Safety cap on pages per layer (nationwide totals are ~11 pages at `PAGE_SIZE`) — guards
/// against an infinite loop if the service ever ignores `resultOffset`.
const MAX_PAGES: usize = 50;
const MAX_ATTEMPTS: u32 = 4;
/// Delay between page requests — the org's ArcGIS quota is request-unit based (cost scales with
/// records returned, not just request count), so a tight loop over full 2000-record pages can
/// trip it well before hitting `MAX_PAGES`. Observed live: ~6000 units/minute.
const PAGE_DELAY: Duration = Duration::from_millis(1500);
/// ArcGIS's 429 quota error carries a "Retry after 60 sec" window.
const QUOTA_BACKOFF: Duration = Duration::from_secs(65);

#[derive(Debug, Default, Serialize)]
struct AirportSurface {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    taxiways: Vec<PolygonRow>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    ramps: Vec<RampRow>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    runways: Vec<PolygonRow>,
}

/// A named pavement polygon — a taxiway or a runway.
#[derive(Debug, Serialize)]
struct PolygonRow {
    name: String,
    rings: Vec<Vec<[f64; 2]>>,
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
    #[serde(rename = "FAA_ID")]
    faa_id: Option<String>,
    #[serde(rename = "DESIGNATOR")]
    designator: Option<String>,
    /// `AM_Runway` only: the runway designator, e.g. `01/19`.
    #[serde(rename = "RWY_ID", default)]
    rwy_id: Option<String>,
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

/// A trimmed, uppercased, non-empty identifier.
fn clean_id(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_ascii_uppercase)
}

fn is_icao(s: &str) -> bool {
    s.len() == 4 && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// The bundled OurAirports airport set (`data/runways.json` keys) used to validate ICAO choices.
fn known_airports() -> HashSet<String> {
    serde_json::from_str::<HashMap<String, serde_json::Value>>(include_str!(
        "../../data/runways.json"
    ))
    .expect("bundled runways.json should parse")
    .into_keys()
    .collect()
}

/// One canonical ICAO per `FAA_ID`, across every feature of both layers. Candidates are the valid
/// 4-char `ICAO_ID`s seen for that `FAA_ID`; the pick is, in order: a candidate in `known` (e.g.
/// `PANC` over `KANC`); the sole candidate (a real airport OurAirports lacks, e.g. `KBVU`);
/// `K` + a 3-char `FAA_ID` if that's in `known` (e.g. blank-ICAO `RUT` → `KRUT`). An `FAA_ID` with
/// several unknown candidates gets no entry — its features keep their own valid `ICAO_ID`.
fn canonical_icaos<'a>(
    features: impl IntoIterator<Item = &'a Feature>,
    known: &HashSet<String>,
) -> HashMap<String, String> {
    let mut candidates: HashMap<String, BTreeSet<String>> = HashMap::new();
    for f in features {
        let Some(faa) = clean_id(f.properties.faa_id.as_deref()) else {
            continue;
        };
        let entry = candidates.entry(faa).or_default();
        if let Some(icao) = clean_id(f.properties.icao_id.as_deref()).filter(|i| is_icao(i)) {
            entry.insert(icao);
        }
    }

    candidates
        .into_iter()
        .filter_map(|(faa, cands)| {
            let pick = cands
                .iter()
                .find(|c| known.contains(*c))
                .cloned()
                .or_else(|| (cands.len() == 1).then(|| cands.first().cloned()).flatten())
                .or_else(|| {
                    let k = format!("K{faa}");
                    (faa.len() == 3 && known.contains(&k)).then_some(k)
                })?;
            Some((faa, pick))
        })
        .collect()
}

/// The ICAO a feature is filed under: its `FAA_ID`'s canonical ICAO, else its own valid `ICAO_ID`.
fn resolve_icao(properties: &Properties, canonical: &HashMap<String, String>) -> Option<String> {
    clean_id(properties.faa_id.as_deref())
        .and_then(|faa| canonical.get(&faa).cloned())
        .or_else(|| clean_id(properties.icao_id.as_deref()).filter(|i| is_icao(i)))
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

/// A polygon feature's designator: `DESIGNATOR`, or `RWY_ID` for a runway.
fn polygon_designator(p: &Properties) -> Option<&str> {
    p.designator
        .as_deref()
        .filter(|d| !d.trim().is_empty())
        .or(p.rwy_id.as_deref())
}

/// Maps `AM_Taxiway` or `AM_Runway` features — both are named pavement polygons — to rows by ICAO.
fn map_polygons(
    features: &[Feature],
    canonical: &HashMap<String, String>,
) -> (HashMap<String, Vec<PolygonRow>>, MapStats) {
    let mut by_icao: HashMap<String, Vec<PolygonRow>> = HashMap::new();
    let mut stats = MapStats::default();

    for f in features {
        let Some(icao) = resolve_icao(&f.properties, canonical) else {
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
        by_icao.entry(icao).or_default().push(PolygonRow {
            name: feature_name(polygon_designator(&f.properties), f.properties.object_id),
            rings: vec![ring],
        });
    }

    (by_icao, stats)
}

fn map_ramps(
    features: &[Feature],
    canonical: &HashMap<String, String>,
) -> (HashMap<String, Vec<RampRow>>, MapStats) {
    let mut by_icao: HashMap<String, Vec<RampRow>> = HashMap::new();
    let mut stats = MapStats::default();

    for f in features {
        let Some(icao) = resolve_icao(&f.properties, canonical) else {
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

/// One offset page of a layer. `orderByFields=OBJECTID` is required for offset paging to be
/// stable — ArcGIS doesn't guarantee a consistent order across pages without one, which would
/// silently duplicate some features and skip others.
fn page_url(base_url: &str, fields: &str, offset: usize) -> String {
    format!(
        "{base_url}/query?where=1%3D1&outFields={fields}&orderByFields=OBJECTID&f=geojson&resultRecordCount={PAGE_SIZE}&resultOffset={offset}"
    )
}

/// What a successfully-received (HTTP 200, valid JSON) page means.
#[derive(Debug)]
enum PageOutcome {
    Page(FeatureCollection),
    /// A quota error: wait this long, then try again.
    Retry(Duration, String),
    /// Any other ArcGIS error: stop the run.
    Fatal(String),
}

fn classify_page(page: FeatureCollection) -> PageOutcome {
    match &page.error {
        None => PageOutcome::Page(page),
        Some(err) if err.code == 429 => {
            PageOutcome::Retry(QUOTA_BACKOFF, format!("quota error: {}", err.message))
        }
        Some(err) => PageOutcome::Fatal(format!("ArcGIS error {}: {}", err.code, err.message)),
    }
}

async fn fetch_page(
    http: &reqwest::Client,
    base_url: &str,
    fields: &str,
    offset: usize,
) -> Result<FeatureCollection, String> {
    let url = page_url(base_url, fields, offset);
    let mut last_err = String::new();

    for attempt in 1..=MAX_ATTEMPTS {
        let mut backoff = Duration::from_secs(2 * attempt as u64);
        match http.get(&url).send().await {
            Ok(r) if r.status().is_success() => match r.json::<FeatureCollection>().await {
                Ok(page) => match classify_page(page) {
                    PageOutcome::Page(page) => return Ok(page),
                    PageOutcome::Retry(wait, msg) => {
                        last_err = format!("{base_url} offset {offset} (attempt {attempt}): {msg}");
                        backoff = wait;
                    }
                    PageOutcome::Fatal(msg) => {
                        return Err(format!("{base_url} offset {offset}: {msg}"));
                    }
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

/// Pages through a layer's full feature set via `fetch(offset)`. Stops on the first empty page
/// rather than comparing against `PAGE_SIZE`, so it adapts automatically if the service enforces a
/// smaller `maxRecordCount` than requested.
async fn fetch_all_features<F, Fut>(mut fetch: F, delay: Duration) -> Result<Vec<Feature>, String>
where
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = Result<FeatureCollection, String>>,
{
    let mut all = Vec::new();
    let mut offset = 0usize;

    for _ in 0..MAX_PAGES {
        let page = fetch(offset).await?;
        let n = page.features.len();
        if n == 0 {
            return Ok(all);
        }
        offset += n;
        all.extend(page.features);
        tokio::time::sleep(delay).await;
    }
    Err(format!(
        "exceeded {MAX_PAGES} pages without an empty page — resultOffset may not be advancing"
    ))
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let http = client();
    let out_path = concat!(env!("CARGO_MANIFEST_DIR"), "/data/faa_surface.json");
    let runways_only = std::env::args().any(|a| a == "--runways-only");

    tracing::info!("fetching AM_Taxiway features");
    let taxiway_features = fetch_all_features(
        |offset| fetch_page(&http, TAXIWAY_URL, SURFACE_FIELDS, offset),
        PAGE_DELAY,
    )
    .await
    .expect("failed to fetch AM_Taxiway features");

    tracing::info!("fetching AM_Apron features");
    let apron_features = fetch_all_features(
        |offset| fetch_page(&http, APRON_URL, SURFACE_FIELDS, offset),
        PAGE_DELAY,
    )
    .await
    .expect("failed to fetch AM_Apron features");

    tracing::info!("fetching AM_Runway features");
    let runway_features = fetch_all_features(
        |offset| fetch_page(&http, RUNWAY_URL, RUNWAY_FIELDS, offset),
        PAGE_DELAY,
    )
    .await
    .expect("failed to fetch AM_Runway features");

    // Keyed across every layer, so a layer's airports land under the same ICAO either way.
    let canonical = canonical_icaos(
        taxiway_features
            .iter()
            .chain(apron_features.iter())
            .chain(runway_features.iter()),
        &known_airports(),
    );
    let (runways_by_icao, runway_stats) = map_polygons(&runway_features, &canonical);

    if runways_only {
        let existing = std::fs::read_to_string(out_path).expect("failed to read existing extract");
        tracing::info!(
            runway_features = runway_features.len(),
            runway_skipped_no_icao = runway_stats.skipped_no_icao,
            "merging runways into the existing extract"
        );
        std::fs::write(out_path, merge_runways(&existing, runways_by_icao))
            .expect("failed to write output file");
        return;
    }

    let (taxiways_by_icao, taxiway_stats) = map_polygons(&taxiway_features, &canonical);
    let (ramps_by_icao, apron_stats) = map_ramps(&apron_features, &canonical);

    let mut extract: HashMap<String, AirportSurface> = HashMap::new();
    for (icao, taxiways) in taxiways_by_icao {
        extract.entry(icao).or_default().taxiways = taxiways;
    }
    for (icao, ramps) in ramps_by_icao {
        extract.entry(icao).or_default().ramps = ramps;
    }
    for (icao, runways) in runways_by_icao {
        extract.entry(icao).or_default().runways = runways;
    }

    tracing::info!(
        airports = extract.len(),
        taxiway_features = taxiway_features.len(),
        apron_features = apron_features.len(),
        runway_features = runway_features.len(),
        taxiway_skipped_no_icao = taxiway_stats.skipped_no_icao,
        apron_skipped_no_icao = apron_stats.skipped_no_icao,
        runway_skipped_no_icao = runway_stats.skipped_no_icao,
        taxiway_multi_ring_seen = taxiway_stats.multi_ring_seen,
        apron_multi_ring_seen = apron_stats.multi_ring_seen,
        "done"
    );

    // Compact, not pretty — this is a large array-of-coordinates file; per-element indentation
    // would multiply its size for a file nobody reads by hand.
    let out = serde_json::to_string(&extract).expect("extract should serialize");
    std::fs::write(out_path, out).expect("failed to write output file");
    tracing::info!(path = out_path, count = extract.len(), "wrote output file");
}

/// Sets each airport's `runways` in an existing extract (`--runways-only`), replacing any previous
/// runways and leaving every other key untouched. An airport with runways but no other data gets
/// an entry of its own; a previous runway set for an airport no longer in `runways` is dropped.
///
/// Panics on an empty `runways`: [`fetch_all_features`] reads an empty first page as "no more
/// features", so a degraded AM_Runway service is indistinguishable from a successful empty run, and
/// merging it would strip every runway from the committed extract.
fn merge_runways(existing: &str, runways: HashMap<String, Vec<PolygonRow>>) -> String {
    assert!(
        !runways.is_empty(),
        "AM_Runway returned no mappable features — refusing to strip runways from the extract"
    );
    let mut extract: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(existing).expect("existing extract should parse");
    for airport in extract.values_mut() {
        if let Some(obj) = airport.as_object_mut() {
            obj.remove("runways");
        }
    }
    for (icao, rows) in runways {
        let airport = extract
            .entry(icao)
            .or_insert_with(|| serde_json::Value::Object(Default::default()));
        airport
            .as_object_mut()
            .expect("extract airports are objects")
            .insert(
                "runways".to_string(),
                serde_json::to_value(rows).expect("runways should serialize"),
            );
    }
    serde_json::to_string(&extract).expect("extract should serialize")
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    fn feature(
        icao: Option<&str>,
        faa: Option<&str>,
        designator: Option<&str>,
        object_id: i64,
        coords: serde_json::Value,
    ) -> Feature {
        serde_json::from_value(serde_json::json!({
            "geometry": { "type": "Polygon", "coordinates": coords },
            "properties": {
                "ICAO_ID": icao,
                "FAA_ID": faa,
                "DESIGNATOR": designator,
                "OBJECTID": object_id,
            }
        }))
        .unwrap()
    }

    fn square() -> serde_json::Value {
        serde_json::json!([[[-77.04, 38.85], [-77.041, 38.851], [-77.04, 38.85]]])
    }

    fn known(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    fn page(n: usize) -> FeatureCollection {
        FeatureCollection {
            features: (0..n)
                .map(|i| feature(Some("KDCA"), Some("DCA"), None, i as i64, square()))
                .collect(),
            error: None,
        }
    }

    #[test]
    fn maps_a_realistic_taxiway_page() {
        let features = vec![
            feature(
                Some("kdca"),
                Some("DCA"),
                Some("M"),
                177,
                serde_json::json!([[[-77.04, 38.85], [-77.041, 38.851], [-77.04, 38.85]]]),
            ),
            feature(
                None,
                None,
                Some("ORPHAN"),
                999,
                serde_json::json!([[[0.0, 0.0]]]),
            ),
            feature(
                Some("KMEM"),
                Some("MEM"),
                None,
                42,
                serde_json::json!([[[-89.97, 35.03]]]),
            ),
        ];
        let canonical = canonical_icaos(&features, &known(&["KDCA", "KMEM"]));

        let (by_icao, stats) = map_polygons(&features, &canonical);

        assert_eq!(stats.skipped_no_icao, 1);
        assert_eq!(stats.multi_ring_seen, 0);

        let kdca = &by_icao["KDCA"][0];
        assert_eq!(kdca.name, "M");
        assert_eq!(
            kdca.rings,
            vec![vec![[38.85, -77.04], [38.851, -77.041], [38.85, -77.04]]]
        );

        // blank DESIGNATOR falls back to a stable FAA-<OBJECTID> name.
        assert_eq!(by_icao["KMEM"][0].name, "FAA-42");
    }

    #[test]
    fn runways_are_named_by_rwy_id() {
        let mut rwy = feature(Some("KDCA"), Some("DCA"), None, 7, square());
        rwy.properties.rwy_id = Some("01/19".into());
        let features = vec![rwy];
        let canonical = canonical_icaos(&features, &known(&["KDCA"]));

        let (by_icao, _) = map_polygons(&features, &canonical);

        assert_eq!(by_icao["KDCA"][0].name, "01/19");
    }

    #[test]
    fn merging_runways_keeps_other_layers_and_replaces_previous_runways() {
        let existing = r#"{"KDCA":{"taxiways":[{"name":"M","rings":[[[1.0,2.0]]]}],"runways":[{"name":"OLD","rings":[]}]},"KBVU":{"runways":[{"name":"GONE","rings":[]}]}}"#;
        let runways = HashMap::from([
            (
                "KDCA".to_string(),
                vec![PolygonRow {
                    name: "01/19".into(),
                    rings: vec![vec![[38.85, -77.04]]],
                }],
            ),
            (
                "KZZZ".to_string(),
                vec![PolygonRow {
                    name: "09/27".into(),
                    rings: vec![vec![[1.0, 2.0]]],
                }],
            ),
        ]);

        let merged: serde_json::Value =
            serde_json::from_str(&merge_runways(existing, runways)).unwrap();

        assert_eq!(merged["KDCA"]["taxiways"][0]["name"], "M");
        assert_eq!(merged["KDCA"]["runways"][0]["name"], "01/19");
        assert_eq!(merged["KDCA"]["runways"].as_array().unwrap().len(), 1);
        assert_eq!(merged["KZZZ"]["runways"][0]["name"], "09/27");
        assert!(merged["KBVU"].get("runways").is_none());
    }

    /// A degraded AM_Runway service returns an empty first page, which `fetch_all_features` reports
    /// as a successful empty run — merging that would strip every runway from the committed extract.
    #[test]
    #[should_panic(expected = "refusing to strip runways")]
    fn merging_an_empty_runway_set_refuses_rather_than_wiping_the_extract() {
        let existing = r#"{"KDCA":{"runways":[{"name":"01/19","rings":[]}]}}"#;
        merge_runways(existing, HashMap::new());
    }

    /// Regression (#230 QA): every real source anomaly seen live in AM_Taxiway/AM_Apron.
    #[test]
    fn keys_split_blank_and_bare_faa_icao_ids_to_one_canonical_airport() {
        let features = vec![
            // ANC split across KANC + PANC → the known PANC.
            feature(Some("KANC"), Some("ANC"), Some("E"), 1, square()),
            feature(Some("PANC"), Some("ANC"), Some("F"), 2, square()),
            // DTW: bare FAA id in ICAO_ID beside the real KDTW.
            feature(Some("DTW"), Some("DTW"), Some("UNK"), 3, square()),
            feature(Some("KDTW"), Some("DTW"), Some("A"), 4, square()),
            // RUT: blank ICAO_ID (taxiways) → K + FAA_ID, since KRUT is a known airport.
            feature(None, Some("RUT"), Some("A"), 5, square()),
            // BVU: a real airport OurAirports lacks — its sole candidate is kept.
            feature(Some("KBVU"), Some("BVU"), Some("A"), 6, square()),
            // 40U: genuinely non-ICAO and not known → skipped.
            feature(None, Some("40U"), Some("A"), 7, square()),
            // A lone bare FAA id with no known K-form is never used as a key.
            feature(Some("QQQ"), Some("QQQ"), Some("A"), 8, square()),
        ];
        let canonical = canonical_icaos(&features, &known(&["PANC", "KDTW", "KRUT"]));

        let (by_icao, stats) = map_polygons(&features, &canonical);

        let mut keys: Vec<&str> = by_icao.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["KBVU", "KDTW", "KRUT", "PANC"]);
        assert_eq!(by_icao["PANC"].len(), 2);
        assert_eq!(by_icao["KDTW"].len(), 2);
        assert_eq!(stats.skipped_no_icao, 2);
    }

    #[test]
    fn several_unknown_candidates_for_one_faa_id_keep_their_own_icao() {
        let features = vec![
            feature(Some("KAAA"), Some("AAA"), None, 1, square()),
            feature(Some("PAAA"), Some("AAA"), None, 2, square()),
        ];
        let canonical = canonical_icaos(&features, &known(&[]));
        assert!(canonical.is_empty());
        let (by_icao, _) = map_polygons(&features, &canonical);
        assert!(by_icao.contains_key("KAAA") && by_icao.contains_key("PAAA"));
    }

    #[test]
    fn the_bundled_known_airport_set_loads() {
        let known = known_airports();
        assert!(known.contains("KDTW") && known.contains("PANC") && known.contains("KRUT"));
    }

    #[test]
    fn maps_aprons_with_kind_always_apron() {
        let features = vec![feature(
            Some("KDCA"),
            Some("DCA"),
            Some("GENERAL AVIATION PARKING"),
            1255,
            serde_json::json!([[[-77.044, 38.846], [-77.043, 38.845], [-77.044, 38.846]]]),
        )];
        let canonical = canonical_icaos(&features, &known(&["KDCA"]));

        let (by_icao, stats) = map_ramps(&features, &canonical);

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
            Some("DCA"),
            Some("A"),
            1,
            serde_json::json!([
                [[-77.04, 38.85], [-77.041, 38.851], [-77.04, 38.85]],
                [[-77.0395, 38.8505]],
            ]),
        )];

        let (by_icao, stats) = map_polygons(&features, &HashMap::new());

        assert_eq!(stats.multi_ring_seen, 1);
        // only the exterior ring's points are kept.
        assert_eq!(by_icao["KDCA"][0].rings.len(), 1);
        assert_eq!(by_icao["KDCA"][0].rings[0].len(), 3);
    }

    #[test]
    fn a_quota_error_retries_after_the_quota_window_and_other_errors_are_fatal() {
        let err = |code: i32| FeatureCollection {
            features: vec![],
            error: Some(ArcgisError {
                code,
                message: "x".into(),
            }),
        };
        assert!(matches!(
            classify_page(err(429)),
            PageOutcome::Retry(d, _) if d == QUOTA_BACKOFF
        ));
        assert!(matches!(classify_page(err(400)), PageOutcome::Fatal(_)));
        // A 200 with an error body must never be treated as a valid (empty) page.
        assert!(!matches!(classify_page(err(429)), PageOutcome::Page(_)));
        assert!(matches!(classify_page(page(2)), PageOutcome::Page(p) if p.features.len() == 2));
    }

    #[test]
    fn page_urls_are_ordered_for_stable_offset_paging() {
        let url = page_url(TAXIWAY_URL, SURFACE_FIELDS, 4000);
        assert!(url.contains("orderByFields=OBJECTID"), "{url}");
        assert!(url.contains("resultOffset=4000"), "{url}");
        assert!(url.contains("FAA_ID"), "{url}");
    }

    #[tokio::test]
    async fn pages_advance_by_records_received_until_an_empty_page() {
        let sizes = RefCell::new(vec![2000, 2000, 700, 0].into_iter());
        let offsets = RefCell::new(Vec::new());
        let all = fetch_all_features(
            |offset| {
                offsets.borrow_mut().push(offset);
                let n = sizes.borrow_mut().next().unwrap();
                async move { Ok(page(n)) }
            },
            Duration::ZERO,
        )
        .await
        .unwrap();

        assert_eq!(all.len(), 4700);
        assert_eq!(*offsets.borrow(), vec![0, 2000, 4000, 4700]);
    }

    #[tokio::test]
    async fn a_short_page_is_not_mistaken_for_the_last_one() {
        // A service capping maxRecordCount below PAGE_SIZE returns short pages mid-layer.
        let sizes = RefCell::new(vec![1000, 1000, 0].into_iter());
        let all = fetch_all_features(
            |_| {
                let n = sizes.borrow_mut().next().unwrap();
                async move { Ok(page(n)) }
            },
            Duration::ZERO,
        )
        .await
        .unwrap();
        assert_eq!(all.len(), 2000);
    }

    #[tokio::test]
    async fn a_service_that_never_returns_an_empty_page_hits_the_page_cap() {
        let calls = RefCell::new(0usize);
        let res = fetch_all_features(
            |_| {
                *calls.borrow_mut() += 1;
                async { Ok(page(1)) }
            },
            Duration::ZERO,
        )
        .await;
        assert!(res.is_err());
        assert_eq!(*calls.borrow(), MAX_PAGES);
    }

    #[tokio::test]
    async fn a_page_error_aborts_the_layer() {
        let res = fetch_all_features(|_| async { Err("boom".to_string()) }, Duration::ZERO).await;
        assert_eq!(res.unwrap_err(), "boom");
    }

    #[test]
    fn coordinates_are_rounded_to_1cm_precision() {
        let features = vec![feature(
            Some("KDCA"),
            Some("DCA"),
            Some("M"),
            1,
            serde_json::json!([[[-77.0416274123456, 38.8520919987654]]]),
        )];
        let (by_icao, _) = map_polygons(&features, &HashMap::new());
        assert_eq!(
            by_icao["KDCA"][0].rings,
            vec![vec![[38.852092, -77.0416274]]]
        );
    }

    #[test]
    fn empty_page_yields_no_rows() {
        let (by_icao, stats) = map_polygons(&[], &HashMap::new());
        assert!(by_icao.is_empty());
        assert_eq!(stats.skipped_no_icao, 0);
    }

    #[test]
    fn a_feature_with_no_rings_is_skipped_and_counted() {
        let features = vec![feature(
            Some("KDCA"),
            Some("DCA"),
            Some("X"),
            1,
            serde_json::json!([]),
        )];
        let (by_icao, stats) = map_polygons(&features, &HashMap::new());
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
            "properties": { "ICAO_ID": "KDCA", "FAA_ID": "DCA", "DESIGNATOR": "X", "OBJECTID": 1 }
        }))
        .unwrap();

        let (by_icao, stats) = map_ramps(&[multipolygon], &HashMap::new());
        assert!(by_icao.is_empty());
        assert_eq!(stats.skipped_no_geometry, 1);
    }
}
