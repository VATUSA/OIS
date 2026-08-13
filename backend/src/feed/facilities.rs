//! Facility → member-airport resolution. A "field" the user enters can be a plain airport
//! (KJFK), an approach/TRACON (N90), or an ARTCC/center (ZNY). Facilities expand to the
//! set of airports underneath them. Data bundled from vatflow's TRACON prefix map + the
//! staffing airport→ARTCC table (see backend/data/facilities.json).

use std::collections::HashMap;
use std::sync::LazyLock;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Facility {
    /// "tracon" (approach) | "artcc" (center).
    pub kind: String,
    /// Member airport ICAOs.
    pub airports: Vec<String>,
}

static FACILITIES: LazyLock<HashMap<String, Facility>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../data/facilities.json")).unwrap_or_default()
});

/// The facility record for `id`, if `id` names a known TRACON/ARTCC.
pub fn lookup(id: &str) -> Option<&'static Facility> {
    FACILITIES.get(id)
}

/// The airports a field resolves to: a facility's members, or the field itself when it's a
/// plain airport (or an unknown id). `id` must be uppercase.
pub fn member_airports(id: &str) -> Vec<String> {
    match FACILITIES.get(id) {
        Some(f) => f.airports.clone(),
        None => vec![id.to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_a_tracon_to_member_airports() {
        let n90 = lookup("N90").expect("N90 should be a known facility");
        assert_eq!(n90.kind, "tracon");
        assert!(n90.airports.contains(&"KJFK".to_string()));
        assert!(n90.airports.contains(&"KEWR".to_string()));
    }

    #[test]
    fn resolves_an_artcc() {
        let zny = lookup("ZNY").expect("ZNY should be a known facility");
        assert_eq!(zny.kind, "artcc");
        assert!(zny.airports.contains(&"KJFK".to_string()));
    }

    #[test]
    fn plain_airport_resolves_to_itself() {
        assert_eq!(member_airports("KBOS"), vec!["KBOS".to_string()]);
    }
}
