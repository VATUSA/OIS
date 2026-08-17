//! Geo helpers for stats collection: great-circle distance and trajectory simplification.
//! Ported from the standalone `stats` ingester (`crates/ingester/src/geo.rs`).

/// Great-circle distance between two lat/lon points, in nautical miles.
pub fn haversine_nm(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R_NM: f64 = 3440.065; // Earth radius in nautical miles
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dphi = (lat2 - lat1).to_radians();
    let dlam = (lon2 - lon1).to_radians();
    let a = (dphi / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dlam / 2.0).sin().powi(2);
    2.0 * R_NM * a.sqrt().asin()
}

/// A single track point kept for simplification.
#[derive(Clone, Copy)]
pub struct TrackPoint {
    pub ts: i64, // unix seconds
    pub lat: f64,
    pub lon: f64,
    pub alt: i32,
}

/// Perpendicular distance (in degrees) from `p` to the line `a`–`b`.
///
/// A flat-earth approximation on lat/lon is fine here: we only need relative magnitudes to decide
/// which points to keep, and flights span small enough arcs that the error is negligible for
/// route-shape preservation.
fn perp_distance(p: &TrackPoint, a: &TrackPoint, b: &TrackPoint) -> f64 {
    let (ax, ay) = (a.lon, a.lat);
    let (bx, by) = (b.lon, b.lat);
    let (px, py) = (p.lon, p.lat);
    let dx = bx - ax;
    let dy = by - ay;
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return ((px - ax).powi(2) + (py - ay).powi(2)).sqrt();
    }
    ((dy * px - dx * py + bx * ay - by * ax).abs()) / len2.sqrt()
}

/// Douglas–Peucker line simplification over lat/lon.
///
/// `epsilon` is a tolerance in degrees (~0.01° ≈ 0.6 nm). Cruise legs collapse to their endpoints
/// while turns and climbs retain their points. Returns the kept points in order, always including
/// the first and last.
pub fn douglas_peucker(points: &[TrackPoint], epsilon: f64) -> Vec<TrackPoint> {
    if points.len() <= 2 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    *keep.last_mut().unwrap() = true;
    dp_recurse(points, 0, points.len() - 1, epsilon, &mut keep);
    points
        .iter()
        .zip(keep)
        .filter_map(|(p, k)| k.then_some(*p))
        .collect()
}

fn dp_recurse(points: &[TrackPoint], first: usize, last: usize, epsilon: f64, keep: &mut [bool]) {
    if last <= first + 1 {
        return;
    }
    let mut max_d = 0.0;
    let mut idx = first;
    for i in (first + 1)..last {
        let d = perp_distance(&points[i], &points[first], &points[last]);
        if d > max_d {
            max_d = d;
            idx = i;
        }
    }
    if max_d > epsilon {
        keep[idx] = true;
        dp_recurse(points, first, idx, epsilon, keep);
        dp_recurse(points, idx, last, epsilon, keep);
    }
}

/// Summary derived from a full track when a flight closes.
pub struct TrackSummary {
    pub duration_s: i32,
    pub distance_nm: f32,
    pub max_altitude: i32,
    pub max_groundspeed: i32,
    /// Simplified path as `[[ts, lat, lon, alt], ...]` for JSONB storage.
    pub path_simplified: serde_json::Value,
}

/// Compute a flight summary + simplified path from an ordered track.
///
/// `speeds` is parallel to `points` (groundspeed at each sample).
pub fn summarize(points: &[TrackPoint], speeds: &[i32], epsilon: f64) -> TrackSummary {
    let duration_s = points
        .last()
        .zip(points.first())
        .map(|(l, f)| (l.ts - f.ts) as i32)
        .unwrap_or(0);

    let distance_nm = points
        .windows(2)
        .map(|w| haversine_nm(w[0].lat, w[0].lon, w[1].lat, w[1].lon))
        .sum::<f64>() as f32;

    let max_altitude = points.iter().map(|p| p.alt).max().unwrap_or(0);
    let max_groundspeed = speeds.iter().copied().max().unwrap_or(0);

    let simplified = douglas_peucker(points, epsilon);
    let path_simplified = serde_json::Value::Array(
        simplified
            .iter()
            .map(|p| {
                serde_json::json!([
                    p.ts,
                    (p.lat * 1e5).round() / 1e5,
                    (p.lon * 1e5).round() / 1e5,
                    p.alt
                ])
            })
            .collect(),
    );

    TrackSummary {
        duration_s,
        distance_nm,
        max_altitude,
        max_groundspeed,
        path_simplified,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn straight_line_collapses_to_endpoints() {
        let pts: Vec<TrackPoint> = (0..10)
            .map(|i| TrackPoint {
                ts: i,
                lat: i as f64,
                lon: i as f64,
                alt: 35000,
            })
            .collect();
        // A perfectly straight diagonal keeps only the two endpoints.
        assert_eq!(douglas_peucker(&pts, 0.01).len(), 2);
    }

    #[test]
    fn a_corner_is_preserved() {
        let pts = vec![
            TrackPoint {
                ts: 0,
                lat: 0.0,
                lon: 0.0,
                alt: 0,
            },
            TrackPoint {
                ts: 1,
                lat: 0.0,
                lon: 1.0,
                alt: 0,
            },
            TrackPoint {
                ts: 2,
                lat: 1.0,
                lon: 1.0,
                alt: 0,
            },
        ];
        // The middle point is a 90° corner — it must survive.
        assert_eq!(douglas_peucker(&pts, 0.01).len(), 3);
    }
}
