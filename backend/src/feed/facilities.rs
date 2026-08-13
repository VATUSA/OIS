//! Facility → member-airport resolution. A "field" the user enters can be a plain airport
//! (KJFK), an approach/TRACON (N90), or an ARTCC/center (ZNY); facilities expand to the
//! airports underneath them.
//!
//! The mapping is refreshed daily from two authoritative VATSIM data projects:
//!   * VATSpy Data Project — airport → ARTCC (FIR) and IATA → ICAO
//!   * SimAware TRACON Project — TRACON → member airports
//!
//! A compile-time snapshot (backend/data/facilities.json) is the offline fallback used
//! until the first successful refresh.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use tokio::sync::RwLock;

const VATSPY_URL: &str =
    "https://raw.githubusercontent.com/vatsimnetwork/vatspy-data-project/master/VATSpy.dat";
const TRACON_TREE_URL: &str =
    "https://api.github.com/repos/vatsimnetwork/simaware-tracon-project/git/trees/main?recursive=1";
const REFRESH_INTERVAL: Duration = Duration::from_secs(24 * 3600);

#[derive(Debug, Clone, Deserialize)]
pub struct Facility {
    /// "tracon" (approach) | "artcc" (center).
    pub kind: String,
    /// Member airport ICAOs.
    pub airports: Vec<String>,
}

pub type FacilityMap = HashMap<String, Facility>;
/// Shared, cheaply-cloneable handle to the current facility map.
pub type FacilityState = Arc<RwLock<FacilityMap>>;

/// The compile-time snapshot, used as the offline fallback and initial value.
pub fn bundled() -> FacilityMap {
    serde_json::from_str(include_str!("../../data/facilities.json")).unwrap_or_default()
}

pub fn new_state() -> FacilityState {
    Arc::new(RwLock::new(bundled()))
}

/// The airports a field resolves to: a facility's members, or the field itself when it's a
/// plain airport (or an unknown id). `id` must be uppercase.
pub fn member_airports(map: &FacilityMap, id: &str) -> Vec<String> {
    match map.get(id) {
        Some(f) => f.airports.clone(),
        None => vec![id.to_string()],
    }
}

/// Spawn the daily refresh job. Fires once at startup, then every 24h; on failure it keeps
/// the current map (the bundled snapshot until a fetch succeeds).
pub fn spawn_refresh(state: FacilityState) {
    tokio::spawn(async move { refresh_loop(state).await });
}

async fn refresh_loop(state: FacilityState) {
    let client = match reqwest::Client::builder()
        .user_agent("ois-backend/0.1 (+https://vatusa.net)")
        .timeout(Duration::from_secs(60))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "facilities: failed to build HTTP client");
            return;
        }
    };

    let mut ticker = tokio::time::interval(REFRESH_INTERVAL);
    loop {
        ticker.tick().await;
        match build_from_sources(&client).await {
            Ok(map) => {
                let n = map.len();
                *state.write().await = map;
                tracing::info!(
                    facilities = n,
                    "facilities refreshed (VATSpy + SimAware TRACON)"
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, "facilities refresh failed; keeping current map");
            }
        }
    }
}

async fn build_from_sources(client: &reqwest::Client) -> Result<FacilityMap, reqwest::Error> {
    // VATSpy: airport -> ARTCC (FIR) and IATA -> ICAO.
    let vatspy = client
        .get(VATSPY_URL)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let (artcc, iata_to_icao) = parse_vatspy(&vatspy);

    // SimAware TRACON project: Boundaries/<TRACON>/<AIRPORT>.json paths.
    let tree: TreeResp = client
        .get(TRACON_TREE_URL)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if tree.truncated.unwrap_or(false) {
        tracing::warn!("SimAware TRACON tree was truncated; facility coverage may be partial");
    }
    let tracon = parse_tracons(&tree, &iata_to_icao);

    let mut map = FacilityMap::new();
    for (id, airports) in tracon {
        map.insert(
            id,
            Facility {
                kind: "tracon".into(),
                airports,
            },
        );
    }
    for (id, airports) in artcc {
        map.entry(id).or_insert(Facility {
            kind: "artcc".into(),
            airports,
        });
    }
    Ok(map)
}

/// Parse VATSpy `[Airports]`: `ICAO|Name|Lat|Lon|IATA|FIR|isPseudo`. Returns
/// (ARTCC id -> member ICAOs, IATA -> ICAO). Pseudo entries are skipped.
fn parse_vatspy(dat: &str) -> (HashMap<String, Vec<String>>, HashMap<String, String>) {
    let mut artcc: HashMap<String, Vec<String>> = HashMap::new();
    let mut iata_to_icao: HashMap<String, String> = HashMap::new();
    let mut in_airports = false;

    for line in dat.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_airports = line.eq_ignore_ascii_case("[Airports]");
            continue;
        }
        if !in_airports || line.is_empty() {
            continue;
        }
        let p: Vec<&str> = line.split('|').collect();
        if p.len() < 7 {
            continue;
        }
        let (icao, iata, fir, pseudo) = (
            p[0].to_ascii_uppercase(),
            p[4].to_ascii_uppercase(),
            p[5],
            p[6],
        );
        if pseudo == "1" {
            continue; // approach/combined pseudo-airport
        }
        if !iata.is_empty() {
            iata_to_icao.insert(iata, icao.clone());
        }
        if let Some(id) = normalize_artcc(fir) {
            artcc.entry(id).or_default().push(icao);
        }
    }
    for v in artcc.values_mut() {
        v.sort();
        v.dedup();
    }
    (artcc, iata_to_icao)
}

/// US ARTCC id from a VATSpy FIR: `KZNY` -> `ZNY`; Alaska/Hawaii/San Juan aliases; else None.
fn normalize_artcc(fir: &str) -> Option<String> {
    match fir.trim().to_ascii_uppercase().as_str() {
        "PAZA" => Some("ZAN".into()),
        "PHZH" => Some("ZHN".into()),
        "TJZS" => Some("ZSU".into()),
        f => {
            let b = f.as_bytes();
            (b.len() == 4 && b[0] == b'K' && b[1] == b'Z').then(|| f[1..].to_string())
        }
    }
}

#[derive(Deserialize)]
struct TreeResp {
    tree: Vec<TreeEntry>,
    #[serde(default)]
    truncated: Option<bool>,
}

#[derive(Deserialize)]
struct TreeEntry {
    path: String,
}

/// TRACON id -> member ICAOs, from `Boundaries/<TRACON>/<AIRPORT>.json` paths. Airport
/// codes are resolved to ICAO via VATSpy's IATA map; unresolvable codes (position/sector
/// files like `LAX_DEP`, `NY`) are dropped.
fn parse_tracons(
    tree: &TreeResp,
    iata_to_icao: &HashMap<String, String>,
) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for e in &tree.tree {
        let Some(rest) = e.path.strip_prefix("Boundaries/") else {
            continue;
        };
        let Some(inner) = rest.strip_suffix(".json") else {
            continue;
        };
        let mut parts = inner.splitn(2, '/');
        let (Some(tid), Some(code)) = (parts.next(), parts.next()) else {
            continue;
        };
        if code.contains('/') {
            continue;
        }
        let code = code.to_ascii_uppercase();
        let icao = if let Some(ic) = iata_to_icao.get(&code) {
            ic.clone()
        } else if code.len() == 4 && code.chars().all(|c| c.is_ascii_alphanumeric()) {
            code.clone() // already an ICAO (e.g. international)
        } else {
            continue; // not a resolvable airport
        };
        out.entry(tid.to_ascii_uppercase()).or_default().push(icao);
    }
    for v in out.values_mut() {
        v.sort();
        v.dedup();
    }
    out.retain(|_, v| !v.is_empty());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_resolves_a_tracon_and_artcc() {
        let map = bundled();
        let n90 = map.get("N90").expect("N90 in bundled snapshot");
        assert_eq!(n90.kind, "tracon");
        assert!(n90.airports.contains(&"KJFK".to_string()));
        assert_eq!(map.get("ZNY").map(|f| f.kind.as_str()), Some("artcc"));
    }

    #[test]
    fn plain_airport_resolves_to_itself() {
        let map = bundled();
        assert_eq!(member_airports(&map, "KBOS"), vec!["KBOS".to_string()]);
    }

    #[test]
    fn artcc_normalization() {
        assert_eq!(normalize_artcc("KZNY").as_deref(), Some("ZNY"));
        assert_eq!(normalize_artcc("PAZA").as_deref(), Some("ZAN"));
        assert_eq!(normalize_artcc("EGTT"), None); // non-US FIR
        assert_eq!(normalize_artcc("KJFK"), None); // not a center
    }

    #[test]
    fn parses_vatspy_airports_section() {
        let dat = "[Airports]\n\
                   KJFK|New York|40.6|-73.7|JFK|KZNY|0\n\
                   KBOS|Boston|42.3|-71.0|BOS|KZBW|0\n\
                   KBOS|Boston A90 TRACON|42.3|-71.0|A90|KZBW|1\n\
                   [FIRs]\n\
                   ignored|line\n";
        let (artcc, iata) = parse_vatspy(dat);
        assert_eq!(artcc.get("ZNY"), Some(&vec!["KJFK".to_string()]));
        assert_eq!(artcc.get("ZBW"), Some(&vec!["KBOS".to_string()])); // pseudo skipped
        assert_eq!(iata.get("JFK"), Some(&"KJFK".to_string()));
    }

    #[test]
    fn parses_tracon_tree_and_resolves_airports() {
        let tree = TreeResp {
            truncated: Some(false),
            tree: vec![
                TreeEntry {
                    path: "Boundaries/N90/JFK.json".into(),
                },
                TreeEntry {
                    path: "Boundaries/N90/EWR.json".into(),
                },
                TreeEntry {
                    path: "Boundaries/N90/NY.json".into(),
                }, // not an airport
                TreeEntry {
                    path: "Boundaries/N90".into(),
                }, // folder entry
            ],
        };
        let iata = HashMap::from([
            ("JFK".to_string(), "KJFK".to_string()),
            ("EWR".to_string(), "KEWR".to_string()),
        ]);
        let tr = parse_tracons(&tree, &iata);
        assert_eq!(
            tr.get("N90"),
            Some(&vec!["KEWR".to_string(), "KJFK".to_string()])
        );
    }
}
