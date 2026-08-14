//! Runway ends per US airport, bundled from OurAirports (`data/runways.json`), for the
//! Runway Balancer. Each field is `[le_id, he_id, le_heading_true, length_ft]`.

use std::collections::HashMap;

use super::runway::RunwayEnd;

/// `(le_id, he_id, le_heading_true, length_ft)`.
type Row = (String, String, i32, i32);

#[derive(Default)]
pub struct RunwayDb {
    fields: HashMap<String, Vec<Row>>,
}

impl RunwayDb {
    /// Parse the bundled runway JSON (embedded at compile time).
    pub fn load() -> Self {
        let fields: HashMap<String, Vec<Row>> =
            serde_json::from_str(include_str!("../../data/runways.json")).unwrap_or_default();
        Self { fields }
    }

    pub fn len(&self) -> usize {
        self.fields.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Both ends of every runway at `icao` (inactive by default). The high end's heading is
    /// the low end's reciprocal.
    pub fn ends_for(&self, icao: &str) -> Vec<RunwayEnd> {
        let mut ends = Vec::new();
        let Some(rows) = self.fields.get(&icao.to_ascii_uppercase()) else {
            return ends;
        };
        for (le, he, le_hdg, len) in rows {
            let pair = format!("{le}/{he}");
            ends.push(RunwayEnd {
                id: le.clone(),
                hdg: ((le_hdg % 360) + 360) % 360,
                len: *len,
                active: false,
                pair: pair.clone(),
            });
            ends.push(RunwayEnd {
                id: he.clone(),
                hdg: ((le_hdg + 180) % 360 + 360) % 360,
                len: *len,
                active: false,
                pair,
            });
        }
        ends
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_bundled_runways_and_expands_ends() {
        let db = RunwayDb::load();
        assert!(
            db.len() > 2000,
            "should load thousands of US fields, got {}",
            db.len()
        );
        let ends = db.ends_for("KJFK");
        // 4 runways → 8 ends.
        assert_eq!(ends.len(), 8);
        let ids: Vec<&str> = ends.iter().map(|e| e.id.as_str()).collect();
        assert!(ids.contains(&"04L") && ids.contains(&"22R"));
        // Reciprocal heading: 04L ~044°true region, 22R ~ +180.
        let le = ends.iter().find(|e| e.id == "04L").unwrap();
        let he = ends.iter().find(|e| e.id == "22R").unwrap();
        assert_eq!(((le.hdg + 180) % 360), he.hdg);
        assert!(db.ends_for("XXXX").is_empty());
    }
}
