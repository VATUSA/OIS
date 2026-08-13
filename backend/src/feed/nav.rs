//! Navigation database — resolves filed-route tokens (fixes / navaids) to coordinates.
//! Bundled from vatflow's FAA-NASR export (`data/nav/{fixes,navaids}.json`), each a
//! `{ NAME: [[lat, lon], ...] }` map (a name may have several candidate coordinates).
//! Airway / SID / STAR expansion is not yet implemented — those tokens are skipped and
//! the surrounding resolved anchors are joined by great circle.

use std::collections::HashMap;

use super::airports::AirportDb;

pub type CoordList = Vec<[f64; 2]>;

#[derive(Default)]
pub struct NavData {
    navaids: HashMap<String, CoordList>,
    fixes: HashMap<String, CoordList>,
}

impl NavData {
    /// Parse the bundled nav JSON (embedded at compile time).
    pub fn load() -> Self {
        let navaids: HashMap<String, CoordList> =
            serde_json::from_str(include_str!("../../data/nav/navaids.json")).unwrap_or_default();
        let fixes: HashMap<String, CoordList> =
            serde_json::from_str(include_str!("../../data/nav/fixes.json")).unwrap_or_default();
        Self { navaids, fixes }
    }

    pub fn len(&self) -> usize {
        self.fixes.len() + self.navaids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Resolve a route token to a coordinate. Priority: airport → navaid → fix; when a
    /// name has multiple candidates, pick the one nearest `prev`.
    pub fn resolve(
        &self,
        token: &str,
        airports: &AirportDb,
        prev: Option<[f64; 2]>,
    ) -> Option<[f64; 2]> {
        if let Some(&(lat, lon)) = airports.get(token) {
            return Some([lat, lon]);
        }
        let cands = self.navaids.get(token).or_else(|| self.fixes.get(token))?;
        Some(nearest(cands, prev))
    }
}

fn nearest(cands: &[[f64; 2]], prev: Option<[f64; 2]>) -> [f64; 2] {
    match prev {
        Some(p) if cands.len() > 1 => *cands
            .iter()
            .min_by(|a, b| {
                dist2(a, &p)
                    .partial_cmp(&dist2(b, &p))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(&cands[0]),
        _ => cands[0],
    }
}

fn dist2(a: &[f64; 2], b: &[f64; 2]) -> f64 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    dx * dx + dy * dy
}
