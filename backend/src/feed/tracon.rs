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
            match fetch(&client).await {
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
    });
}

async fn fetch(client: &reqwest::Client) -> Result<TraconData, reqwest::Error> {
    let text = client
        .get(BOUNDARIES_URL)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse(&text))
}

#[derive(Deserialize)]
struct FeatureCollection {
    #[serde(default)]
    features: Vec<Feature>,
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

fn parse(src: &str) -> TraconData {
    let fc: FeatureCollection = serde_json::from_str(src).unwrap_or(FeatureCollection {
        features: Vec::new(),
    });
    let mut features = Vec::new();
    for f in fc.features {
        let Some(id) = f.properties.id else { continue };
        let rings = outer_rings(&id, &f.geometry);
        if rings.is_empty() {
            continue;
        }
        let prefixes: Vec<String> = f
            .properties
            .prefix
            .iter()
            .map(|p| p.to_ascii_uppercase())
            .collect();
        if prefixes.is_empty() {
            continue;
        }
        let label = match (f.properties.label_lat, f.properties.label_lon) {
            (Some(lat), Some(lon)) => Some([lat, lon]),
            _ => None,
        };
        features.push(TraconFeature {
            id: id.to_ascii_uppercase(),
            name: f.properties.name,
            prefixes,
            suffix: f.properties.suffix.map(|s| s.to_ascii_uppercase()),
            label,
            rings,
        });
    }
    TraconData::build(features)
}

/// Outer ring(s) as `[lat, lon]` from a Polygon or MultiPolygon (holes ignored), with off-globe
/// vertices removed (see [`sanitize_ring`]).
fn outer_rings(id: &str, geom: &Geometry) -> Vec<Vec<[f64; 2]>> {
    let outers: Vec<Vec<[f64; 2]>> = match geom.gtype.as_str() {
        "Polygon" => serde_json::from_value::<Vec<Vec<[f64; 2]>>>(geom.coordinates.clone())
            .ok()
            .and_then(|rings| rings.into_iter().next())
            .into_iter()
            .collect(),
        "MultiPolygon" => {
            serde_json::from_value::<Vec<Vec<Vec<[f64; 2]>>>>(geom.coordinates.clone())
                .ok()
                .map(|polys| {
                    polys
                        .into_iter()
                        .filter_map(|rings| rings.into_iter().next())
                        .collect()
                })
                .unwrap_or_default()
        }
        _ => Vec::new(),
    };
    outers
        .into_iter()
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
        let src = r#"{"type":"FeatureCollection","features":[
          {"type":"Feature","properties":{"id":"NCT","prefix":["SFO"],"suffix":"APP","name":"NorCal Approach","label_lat":36.4,"label_lon":-122.1},
           "geometry":{"type":"Polygon","coordinates":[[[-122.5,37.0],[-122.0,37.0],[-122.0,37.5],[-122.5,37.5],[-122.5,37.0]]]}},
          {"type":"Feature","properties":{"id":"NCT","prefix":["SFO","OAK"],"suffix":"DEP","name":"NorCal Departure"},
           "geometry":{"type":"Polygon","coordinates":[[[-122.6,37.0],[-122.0,37.0],[-122.0,37.6],[-122.6,37.6],[-122.6,37.0]]]}},
          {"type":"Feature","properties":{"id":"SCT","prefix":["SCT"],"name":"SoCal Approach"},
           "geometry":{"type":"Polygon","coordinates":[[[-118.5,33.0],[-118.0,33.0],[-118.0,33.5],[-118.5,33.5],[-118.5,33.0]]]}}
        ]}"#;
        parse(src)
    }

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
        let d = parse(src);
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
}
