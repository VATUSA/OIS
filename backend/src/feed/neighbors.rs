//! ARTCC Tier-1 adjacency — which centers directly border a given center. Sourced from a bundled
//! snapshot of vatflow's `artcc-neighbors.json` (a symmetric neighbor graph covering the US ARTCCs
//! plus Canadian/oceanic FIRs). Used to fan out FNO (Friday) support requests to a host ARTCC's
//! immediate neighbors.

use std::{
    collections::{HashMap, HashSet},
    sync::LazyLock,
};

/// The bundled adjacency map, parsed once. Keys/values use the dataset's ids (Honolulu = `ZHN`).
static NEIGHBORS: LazyLock<HashMap<String, Vec<String>>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../data/artcc-neighbors.json")).unwrap_or_default()
});

/// OIS uses `HCF` for Honolulu; the adjacency dataset uses `ZHN`. Normalize in both directions.
fn to_dataset(id: &str) -> &str {
    if id == "HCF" { "ZHN" } else { id }
}
fn from_dataset(id: &str) -> &str {
    if id == "ZHN" { "HCF" } else { id }
}

/// The Tier-1 (directly bordering) ARTCCs of `host`, restricted to facilities OIS actually knows
/// (`known`) and excluding the host itself. `known` should be OIS's active facility ids — that filter
/// drops the Canadian/oceanic FIRs present in the dataset. Empty if the host has no OIS neighbors.
pub fn tier1(host: &str, known: &HashSet<String>) -> Vec<String> {
    let host = host.to_ascii_uppercase();
    let Some(list) = NEIGHBORS.get(to_dataset(&host)) else {
        return Vec::new();
    };
    list.iter()
        .map(|n| from_dataset(n).to_string())
        .filter(|n| *n != host && known.contains(n))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known() -> HashSet<String> {
        // OIS's 22 facilities.
        [
            "ZBW", "ZDC", "ZNY", "ZOB", "ZID", "ZJX", "ZMA", "ZTL", "ZAB", "ZFW", "ZHU", "ZME",
            "ZAU", "ZDV", "ZKC", "ZMP", "ZAN", "HCF", "ZLA", "ZLC", "ZOA", "ZSE",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    }

    #[test]
    fn tier1_filters_to_known_facilities() {
        // ZBW borders ZQM/ZUL (Canadian) in the dataset — those must be dropped.
        let mut got = tier1("ZBW", &known());
        got.sort();
        assert_eq!(got, ["ZDC", "ZNY", "ZOB"]);
    }

    #[test]
    fn honolulu_alias_has_no_neighbors() {
        assert!(tier1("HCF", &known()).is_empty());
    }

    #[test]
    fn unknown_host_is_empty() {
        assert!(tier1("ZZZ", &known()).is_empty());
    }
}
