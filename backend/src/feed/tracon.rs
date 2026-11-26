//! SimAware TRACON boundaries — the polygon areas drawn for online approach/departure
//! controllers (the "ATC" map layer). Fetched once at startup and refreshed daily from the
//! SimAware TRACON Project's combined release asset, held in memory behind an `ArcSwap`.
//!
//! Each feature carries `prefix` (airport/TRACON codes the position callsign can start with)
//! and an optional `suffix` (`APP`/`DEP`); a controller callsign matches when its first
//! `_`-segment is one of the prefixes and — when a suffix is present — its last segment is
//! that suffix. See <https://github.com/vatsimnetwork/simaware-tracon-project>.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use serde::Deserialize;

/// Latest-release asset (GitHub redirects `latest` to the current tag, so this URL is stable).
const BOUNDARIES_URL: &str = "https://github.com/vatsimnetwork/simaware-tracon-project/releases/latest/download/TRACONBoundaries.geojson";
const REFRESH_INTERVAL: Duration = Duration::from_secs(24 * 3600);

pub type TraconState = Arc<ArcSwap<TraconData>>;

/// One matchable TRACON boundary (a TRACON usually has several — one per airport/sub-sector).
#[derive(Debug, Clone)]
pub struct TraconFeature {
    /// TRACON id, e.g. `NCT`, `SCT`, `N90`.
    pub id: String,
    pub name: Option<String>,
    /// Callsign prefixes that select this feature (airport codes, or the TRACON id itself).
    pub prefixes: Vec<String>,
    /// When set, the callsign's last segment must equal this (`APP` vs `DEP` sub-areas).
    pub suffix: Option<String>,
    /// Preferred label anchor `[lat, lon]`, if the source provides one.
    pub label: Option<[f64; 2]>,
    /// Outer rings, each `[lat, lon]` (a MultiPolygon yields several).
    pub rings: Vec<Vec<[f64; 2]>>,
}

#[derive(Debug, Default)]
pub struct TraconData {
    features: Vec<TraconFeature>,
    /// Uppercase prefix → indices into `features`.
    by_prefix: HashMap<String, Vec<usize>>,
}

impl TraconData {
    fn build(features: Vec<TraconFeature>) -> Self {
        let mut by_prefix: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, f) in features.iter().enumerate() {
            for p in &f.prefixes {
                by_prefix.entry(p.clone()).or_default().push(i);
            }
        }
        Self {
            features,
            by_prefix,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.features.is_empty()
    }

    /// The best boundary for an approach/departure callsign, or `None` if unmatched.
    /// Prefers a feature whose suffix matches the callsign over a generic (suffix-less) one.
    pub fn match_callsign(&self, callsign: &str) -> Option<&TraconFeature> {
        let up = callsign.to_ascii_uppercase();
        let mut segs = up.split('_');
        let first = segs.next()?;
        let last = up.rsplit('_').next().unwrap_or("");
        let cands = self.by_prefix.get(first)?;
        let mut best: Option<(&TraconFeature, i32)> = None;
        for &i in cands {
            let f = &self.features[i];
            let score = match &f.suffix {
                Some(s) if s == last => 2,
                Some(_) => continue, // suffix present but doesn't match this callsign
                None => 1,
            };
            if best.is_none_or(|(_, b)| score > b) {
                best = Some((f, score));
            }
        }
        best.map(|(f, _)| f)
    }
}

pub fn new_state() -> TraconState {
    Arc::new(ArcSwap::from_pointee(TraconData::default()))
}

/// Fetch once at startup, then refresh daily. On failure the current data is kept (empty
/// until the first success — approach areas simply don't draw yet).
pub fn spawn_refresh(state: TraconState) {
    tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .user_agent("ois-backend/0.1 (+https://vatusa.net)")
            .timeout(Duration::from_secs(60))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(error = %e, "tracon: failed to build HTTP client");
                return;
            }
        };
        let mut ticker = tokio::time::interval(REFRESH_INTERVAL);
        loop {
            ticker.tick().await;
            apply_refresh(&state, fetch(&client).await);
        }
    });
}

/// Parse and install one download. A parsed set replaces the current one; a failed download, or a
/// release with no usable feature, keeps it.
fn apply_refresh(state: &TraconState, downloaded: Result<String, reqwest::Error>) {
    let parsed = downloaded
        .map_err(FetchError::from)
        .and_then(|text| parse(&text).map_err(FetchError::from));
    match parsed {
        Ok(data) => {
            tracing::info!(
                tracons = data.features.len(),
                "SimAware TRACON boundaries loaded"
            );
            state.store(Arc::new(data));
        }
        Err(e) => {
            tracing::warn!(error = %e, "tracon: refresh failed; keeping current boundaries");
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum FetchError {
    #[error("download failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Parse(#[from] ParseError),
}

#[derive(Debug, thiserror::Error)]
enum ParseError {
    #[error("not a GeoJSON FeatureCollection: {0}")]
    Document(#[from] serde_json::Error),
    #[error("no usable TRACON feature among {total} entries")]
    NoUsableFeatures { total: usize },
}

async fn fetch(client: &reqwest::Client) -> Result<String, reqwest::Error> {
    client
        .get(BOUNDARIES_URL)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await
}

#[derive(Deserialize)]
struct FeatureCollection {
    /// Raw entries, each converted to a [`Feature`] on its own, so one entry of another shape (a
    /// nested `FeatureCollection`, #1120) is skipped instead of failing the whole document.
    #[serde(default)]
    features: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct Feature {
    properties: Props,
    geometry: Geometry,
}

#[derive(Deserialize)]
struct Props {
    id: Option<String>,
    #[serde(default)]
    prefix: Vec<String>,
    #[serde(default)]
    suffix: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    label_lat: Option<f64>,
    #[serde(default)]
    label_lon: Option<f64>,
}

#[derive(Deserialize)]
struct Geometry {
    #[serde(rename = "type")]
    gtype: String,
    #[serde(default)]
    coordinates: serde_json::Value,
}

/// Parse the combined boundaries document. Every entry that yields no boundary is skipped and
/// counted in one warning: one that does not parse as a TRACON `Feature` (`malformed`), and one that
/// parses but has no id, prefix or drawable ring (`unusable`). A document that is not a feature
/// collection, or one with no usable feature, is an error, so the refresh keeps the boundaries it
/// holds.
fn parse(src: &str) -> Result<TraconData, ParseError> {
    let fc: FeatureCollection = serde_json::from_str(src)?;
    let total = fc.features.len();
    let mut malformed = 0usize;
    let mut first_malformed: Option<(usize, serde_json::Error)> = None;
    let mut features = Vec::new();
    for (index, value) in fc.features.into_iter().enumerate() {
        match serde_json::from_value::<Feature>(value) {
            Ok(f) => features.extend(convert(f)),
            Err(e) => {
                malformed += 1;
                first_malformed.get_or_insert((index, e));
            }
        }
    }
    let unusable = total - malformed - features.len();
    if malformed + unusable > 0 {
        let (first_index, first_error) = match &first_malformed {
            Some((i, e)) => (Some(*i), Some(e.to_string())),
            None => (None, None),
        };
        tracing::warn!(
            malformed,
            unusable,
            total,
            first_malformed_index = first_index,
            first_malformed_error = first_error.as_deref(),
            "tracon: skipped entries that yield no boundary"
        );
    }
    if features.is_empty() {
        return Err(ParseError::NoUsableFeatures { total });
    }
    Ok(TraconData::build(features))
}

/// One source feature as a matchable boundary, or `None` without an id, a prefix or a drawable ring.
fn convert(f: Feature) -> Option<TraconFeature> {
    let id = f.properties.id?;
    let rings = outer_rings(&id, &f.geometry);
    if rings.is_empty() {
        return None;
    }
    let prefixes: Vec<String> = f
        .properties
        .prefix
        .iter()
        .map(|p| p.to_ascii_uppercase())
        .collect();
    if prefixes.is_empty() {
        return None;
    }
    let label = match (f.properties.label_lat, f.properties.label_lon) {
        (Some(lat), Some(lon)) => Some([lat, lon]),
        _ => None,
    };
    Some(TraconFeature {
        id: id.to_ascii_uppercase(),
        name: f.properties.name,
        prefixes,
        suffix: f.properties.suffix.map(|s| s.to_ascii_uppercase()),
        label,
        rings,
    })
}

/// Outer ring(s) as `[lat, lon]` from a Polygon or MultiPolygon, with off-globe vertices removed
/// (see [`sanitize_ring`]).
///
/// Interior rings (holes) are dropped, because `TraconData` carries a flat list of rings with no
/// hole structure — representing them would mean reshaping that type, which is well beyond what a
/// boundary reader should decide. Dropping them **is** a silent reshaping of a donut or multi-part
/// TRACON, though, so it is logged: a shape drawn solid when it should have a hole is otherwise
/// indistinguishable from correct data (VATUSA/OIS#481).
fn outer_rings(id: &str, geom: &Geometry) -> Vec<Vec<[f64; 2]>> {
    let polys: Vec<Vec<Vec<[f64; 2]>>> = match geom.gtype.as_str() {
        "Polygon" => serde_json::from_value::<Vec<Vec<[f64; 2]>>>(geom.coordinates.clone())
            .ok()
            .into_iter()
            .collect(),
        "MultiPolygon" => {
            serde_json::from_value::<Vec<Vec<Vec<[f64; 2]>>>>(geom.coordinates.clone())
                .unwrap_or_default()
        }
        _ => Vec::new(),
    };
    let holes: usize = polys
        .iter()
        .map(|rings| rings.len().saturating_sub(1))
        .sum();
    if holes > 0 {
        tracing::warn!(
            tracon = id,
            holes,
            "tracon: dropped interior ring(s); the area will be drawn solid"
        );
    }
    polys
        .into_iter()
        .filter_map(|rings| rings.into_iter().next())
        .filter_map(|ring| sanitize_ring(id, ring))
        .collect()
}

/// A GeoJSON `[lon, lat]` ring as `[lat, lon]`, keeping only finite, on-globe vertices — a stray
/// bad vertex otherwise draws a map-spanning wedge (VATUSA/OIS#318). `None` when fewer than 3
/// vertices survive. Anything dropped is logged with the TRACON id so dirty upstream features show.
fn sanitize_ring(id: &str, ring: Vec<[f64; 2]>) -> Option<Vec<[f64; 2]>> {
    let total = ring.len();
    let kept: Vec<[f64; 2]> = ring
        .into_iter()
        .filter(|[lon, lat]| {
            lon.is_finite() && lat.is_finite() && lon.abs() <= 180.0 && lat.abs() <= 90.0
        })
        .map(|[lon, lat]| [lat, lon])
        .collect();
    let dropped = total - kept.len();
    if dropped > 0 {
        tracing::warn!(
            tracon = id,
            dropped,
            "tracon: dropped off-globe ring vertices"
        );
    }
    if kept.len() < 3 {
        if total > 0 {
            tracing::warn!(
                tracon = id,
                vertices = kept.len(),
                "tracon: dropped a ring with too few valid vertices"
            );
        }
        return None;
    }
    Some(kept)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> TraconData {
        parse(SAMPLE).expect("sample parses")
    }

    const SAMPLE: &str = r#"{"type":"FeatureCollection","features":[
          {"type":"Feature","properties":{"id":"NCT","prefix":["SFO"],"suffix":"APP","name":"NorCal Approach","label_lat":36.4,"label_lon":-122.1},
           "geometry":{"type":"Polygon","coordinates":[[[-122.5,37.0],[-122.0,37.0],[-122.0,37.5],[-122.5,37.5],[-122.5,37.0]]]}},
          {"type":"Feature","properties":{"id":"NCT","prefix":["SFO","OAK"],"suffix":"DEP","name":"NorCal Departure"},
           "geometry":{"type":"Polygon","coordinates":[[[-122.6,37.0],[-122.0,37.0],[-122.0,37.6],[-122.6,37.6],[-122.6,37.0]]]}},
          {"type":"Feature","properties":{"id":"SCT","prefix":["SCT"],"name":"SoCal Approach"},
           "geometry":{"type":"Polygon","coordinates":[[[-118.5,33.0],[-118.0,33.0],[-118.0,33.5],[-118.5,33.5],[-118.5,33.0]]]}}
        ]}"#;

    #[test]
    fn matches_suffix_specific_feature() {
        let d = sample();
        let app = d.match_callsign("SFO_APP").expect("SFO_APP matches");
        assert_eq!(app.suffix.as_deref(), Some("APP"));
        let dep = d.match_callsign("SFO_DEP").expect("SFO_DEP matches");
        assert_eq!(dep.suffix.as_deref(), Some("DEP"));
    }

    #[test]
    fn three_segment_callsign_matches_by_first_and_last() {
        let d = sample();
        let m = d
            .match_callsign("SFO_38_APP")
            .expect("sector callsign matches");
        assert_eq!(m.suffix.as_deref(), Some("APP"));
    }

    #[test]
    fn tracon_id_prefix_matches_generic_feature() {
        let d = sample();
        let m = d
            .match_callsign("SCT_APP")
            .expect("SCT_APP matches generic SoCal");
        assert_eq!(m.id, "SCT");
        assert!(m.suffix.is_none());
    }

    #[test]
    fn off_globe_vertices_are_dropped_and_a_ring_left_too_small_is_dropped() {
        let src = r#"{"type":"FeatureCollection","features":[
          {"type":"Feature","properties":{"id":"BAD","prefix":["BAD"]},
           "geometry":{"type":"Polygon","coordinates":[[[-80.0,40.0],[-80.0,41.0],[-79.0,41.0],[-79.0,400.0],[-80.0,40.0]]]}},
          {"type":"Feature","properties":{"id":"GONE","prefix":["GONE"]},
           "geometry":{"type":"Polygon","coordinates":[[[-80.0,40.0],[500.0,41.0],[-79.0,95.0]]]}}
        ]}"#;
        let d = parse(src).expect("BAD keeps a ring");
        let bad = d
            .match_callsign("BAD_APP")
            .expect("BAD keeps its valid vertices");
        assert_eq!(
            bad.rings,
            vec![vec![
                [40.0, -80.0],
                [41.0, -80.0],
                [41.0, -79.0],
                [40.0, -80.0]
            ]]
        );
        // Only one valid vertex left: no ring, so the feature is skipped entirely.
        assert!(d.match_callsign("GONE_APP").is_none());
    }

    #[test]
    fn unknown_prefix_is_unmatched() {
        assert!(sample().match_callsign("ZZZ_APP").is_none());
    }

    /// Bytes cut from SimAware release v1.2.15's `TRACONBoundaries.geojson`: its header, the nested
    /// Luxembourg `FeatureCollection` that release put at `features[230]` with its two neighbors,
    /// and the IND feature.
    const V1_2_15_EXCERPT: &str =
        include_str!("../../../fixtures/simaware-tracon-v1.2.15-excerpt.geojson");

    #[test]
    fn a_nested_feature_collection_is_skipped_and_the_valid_features_load() {
        let d = parse(V1_2_15_EXCERPT).expect("the valid features beside the nested one load");
        let ind = d
            .match_callsign("IND_APP")
            .expect("IND_APP matches its polygon");
        assert_eq!(ind.id, "IND");
        assert!(!ind.rings.is_empty(), "IND keeps its ring");
        assert!(
            d.match_callsign("EKYT_APP").is_some(),
            "the neighbor before it loads"
        );
        assert!(
            d.match_callsign("ELM_APP").is_some(),
            "the neighbor after it loads"
        );
        assert!(
            d.match_callsign("ELLX_APP").is_none(),
            "the nested collection is skipped, not recovered"
        );
        assert_eq!(d.features.len(), 3);
    }

    #[test]
    fn a_document_that_is_not_a_feature_collection_is_an_error() {
        for (case, src) in [
            (
                "truncated",
                r#"{"type":"FeatureCollection","features":[{"type":"Feature""#,
            ),
            ("not json", "<html>rate limited</html>"),
            (
                "features is not an array",
                r#"{"type":"FeatureCollection","features":{}}"#,
            ),
        ] {
            assert!(
                matches!(parse(src), Err(ParseError::Document(_))),
                "{case}: expected a document error"
            );
        }
    }

    #[test]
    fn a_document_with_no_usable_feature_is_an_error() {
        for (case, src, total) in [
            ("empty", r#"{"type":"FeatureCollection","features":[]}"#, 0),
            ("no features key", r#"{"type":"FeatureCollection"}"#, 0),
            (
                "only a nested collection",
                r#"{"type":"FeatureCollection","features":[{"type":"FeatureCollection","features":[]}]}"#,
                1,
            ),
            (
                "only a feature without a prefix",
                r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"id":"X"},
                  "geometry":{"type":"Polygon","coordinates":[[[-80.0,40.0],[-80.0,41.0],[-79.0,41.0],[-80.0,40.0]]]}}]}"#,
                1,
            ),
        ] {
            match parse(src) {
                Err(ParseError::NoUsableFeatures { total: t }) => assert_eq!(t, total, "{case}"),
                other => panic!(
                    "{case}: expected NoUsableFeatures, got {:?}",
                    other.map(|d| d.features.len())
                ),
            }
        }
    }

    /// Drives `apply_refresh` with downloaded text, the step `spawn_refresh` runs after each fetch.
    #[test]
    fn a_failed_refresh_keeps_the_boundaries_already_loaded() {
        let state = new_state();
        apply_refresh(&state, Ok(SAMPLE.to_owned()));
        assert!(
            state.load().match_callsign("SFO_APP").is_some(),
            "control: the load landed"
        );

        apply_refresh(
            &state,
            Ok(r#"{"type":"FeatureCollection","features":[]}"#.to_owned()),
        );
        assert!(
            state.load().match_callsign("SFO_APP").is_some(),
            "an empty release keeps the current boundaries"
        );

        apply_refresh(&state, Ok("<html>".to_owned()));
        assert!(
            state.load().match_callsign("SFO_APP").is_some(),
            "a malformed release keeps the current boundaries"
        );

        apply_refresh(&state, Ok(V1_2_15_EXCERPT.to_owned()));
        let now = state.load();
        assert!(
            now.match_callsign("IND_APP").is_some(),
            "a good release replaces them"
        );
        assert!(
            now.match_callsign("SFO_APP").is_none(),
            "the old set is gone"
        );
    }

    /// A `MakeWriter` that keeps everything the fmt layer writes.
    #[derive(Clone, Default)]
    struct Buffer(Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .expect("the buffer's mutex is only held for a write")
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// The lines `parse` logs at WARN for `src`, as plain text.
    fn parse_warnings(src: &str) -> Vec<String> {
        let buffer = Buffer::default();
        let writer = buffer.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(tracing::Level::WARN)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || parse(src))
            .expect("every document these tests log from parses");
        let bytes = buffer.0.lock().expect("no writer holds the lock").clone();
        String::from_utf8(bytes)
            .expect("the fmt layer writes UTF-8")
            .lines()
            .filter(|l| l.contains("tracon: skipped entries that yield no boundary"))
            .map(str::to_owned)
            .collect()
    }

    /// One entry per way an entry yields no boundary, beside one that loads (lowercase, labelled).
    const MIXED: &str = r#"{"type":"FeatureCollection","features":[
      {"type":"Feature","properties":{"id":"nct","prefix":["sfo"],"suffix":"app","label_lat":36.4,"label_lon":-122.1},
       "geometry":{"type":"Polygon","coordinates":[[[-122.5,37.0],[-122.0,37.0],[-122.0,37.5],[-122.5,37.0]]]}},
      {"type":"Feature","properties":{"prefix":["NOID"]},
       "geometry":{"type":"Polygon","coordinates":[[[-80.0,40.0],[-80.0,41.0],[-79.0,41.0],[-80.0,40.0]]]}},
      {"type":"Feature","properties":{"id":"PT","prefix":["PT"]},
       "geometry":{"type":"Point","coordinates":[-80.0,40.0]}},
      {"type":"Feature","properties":{"id":"STR","prefix":"STR"},
       "geometry":{"type":"Polygon","coordinates":[[[-80.0,40.0],[-80.0,41.0],[-79.0,41.0],[-80.0,40.0]]]}},
      {"type":"FeatureCollection","features":[]}
    ]}"#;

    #[test]
    fn every_skipped_entry_is_counted_in_one_warning() {
        let excerpt = parse_warnings(V1_2_15_EXCERPT);
        assert_eq!(excerpt.len(), 1, "one skip warning: {excerpt:?}");
        for field in [
            "malformed=1",
            "unusable=0",
            "total=4",
            "first_malformed_index=1",
        ] {
            assert!(excerpt[0].contains(field), "{field} in {}", excerpt[0]);
        }
        assert!(
            excerpt[0].contains("missing field `properties`"),
            "the first failure's error: {}",
            excerpt[0]
        );

        let mixed = parse_warnings(MIXED);
        assert_eq!(mixed.len(), 1, "one skip warning: {mixed:?}");
        for field in [
            "malformed=2",
            "unusable=2",
            "total=5",
            "first_malformed_index=3",
        ] {
            assert!(mixed[0].contains(field), "{field} in {}", mixed[0]);
        }
        assert!(
            mixed[0].contains("expected a sequence"),
            "the first failure's error, not the last's: {}",
            mixed[0]
        );

        let unusable_only = parse_warnings(
            r#"{"type":"FeatureCollection","features":[
              {"type":"Feature","properties":{"id":"SCT","prefix":["SCT"]},
               "geometry":{"type":"Polygon","coordinates":[[[-118.5,33.0],[-118.0,33.0],[-118.0,33.5],[-118.5,33.0]]]}},
              {"type":"Feature","properties":{"id":"PT","prefix":["PT"]},
               "geometry":{"type":"Point","coordinates":[-80.0,40.0]}}
            ]}"#,
        );
        assert_eq!(
            unusable_only.len(),
            1,
            "one skip warning: {unusable_only:?}"
        );
        for field in ["malformed=0", "unusable=1", "total=2"] {
            assert!(
                unusable_only[0].contains(field),
                "{field} in {}",
                unusable_only[0]
            );
        }

        assert!(
            parse_warnings(SAMPLE).is_empty(),
            "a document where every entry loads logs no skip warning"
        );
    }

    #[test]
    fn a_feature_is_uppercased_and_keeps_its_label_while_an_id_less_one_is_dropped() {
        let d = parse(MIXED).expect("the lowercase feature loads");
        assert_eq!(
            d.features.len(),
            1,
            "only the lowercase feature yields a boundary"
        );
        let f = d
            .match_callsign("SFO_APP")
            .expect("lowercase prefix and suffix match an uppercase callsign");
        assert_eq!(f.id, "NCT");
        assert_eq!(f.prefixes, vec!["SFO".to_owned()]);
        assert_eq!(f.suffix.as_deref(), Some("APP"));
        assert_eq!(f.label, Some([36.4, -122.1]));
        assert!(d.match_callsign("NOID_APP").is_none(), "no id, no boundary");
        assert!(
            d.match_callsign("PT_APP").is_none(),
            "a Point draws no ring"
        );
    }
}
