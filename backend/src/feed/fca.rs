//! FCA geometry — resolve a filed route to a great-circle path and test whether it
//! crosses an FCA polyline. Ported from vatflow's `fca-metering.js` / `route-engine.js`
//! crossing logic (segment intersection in a local tangent plane).

use super::airports::AirportDb;
use super::flow::gc_dist;
use super::nav::NavData;

const NM_PER_DEG: f64 = 60.0;
const DENSIFY_STEP_NM: f64 = 40.0;

/// Where a route crosses an FCA line.
#[derive(Debug, Clone)]
pub struct FcaCrossing {
    pub lat: f64,
    pub lon: f64,
    /// Distance along the (remaining, for airborne) route to the crossing, nm.
    pub along_nm: f64,
}

/// One resolved route anchor: a fix's name (empty for an unnamed procedure-leg point) and its
/// position. Kept internal to this module — `route_path`'s public shape stays `[f64; 2]`; the
/// name only matters to [`route_path_named`].
#[derive(Clone)]
struct NamedAnchor {
    name: String,
    ll: [f64; 2],
}

/// Resolve the filed route to anchors: departure → expanded enroute
/// (fixes/navaids/airways/SID/STAR) → arrival, via the nav engine ([`NavData::build_anchors`]).
fn route_anchors(
    nav: &NavData,
    airports: &AirportDb,
    dep: &str,
    arr: &str,
    route: &str,
) -> Vec<NamedAnchor> {
    nav.build_anchors(airports, dep, arr, route)
        .anchors
        .into_iter()
        .map(|a| NamedAnchor {
            name: a.name,
            ll: a.ll,
        })
        .collect()
}

/// The full filed route as `(name, lat, lon)` anchors + the unresolved tokens. Anchors
/// with an empty name (e.g. unnamed procedure legs) are dropped so callers can label them.
pub fn full_route_named(
    nav: &NavData,
    airports: &AirportDb,
    dep: &str,
    arr: &str,
    route: &str,
) -> (Vec<(String, f64, f64)>, Vec<String>) {
    let res = nav.build_anchors(airports, dep, arr, route);
    let waypoints = res
        .anchors
        .into_iter()
        .filter(|a| !a.name.is_empty())
        .map(|a| (a.name, a.ll[0], a.ll[1]))
        .collect();
    (waypoints, res.unresolved)
}

/// The index of the first anchor still ahead of `(lat, lon)` along the polyline — a nearest-leg
/// projection (perpendicular distance in a local frame centred on the aircraft) that's
/// heading-independent, so it's valid at any speed: parked, taxiing, or flying.
fn project_forward_index(anchors: &[NamedAnchor], lat: f64, lon: f64) -> usize {
    let mut best_leg = 0usize;
    let mut best_xt = f64::MAX;
    let mut best_along = 0.0;
    for i in 0..anchors.len() - 1 {
        let a = local([lat, lon], anchors[i].ll);
        let b = local([lat, lon], anchors[i + 1].ll);
        let (abx, aby) = (b.0 - a.0, b.1 - a.1);
        let (apx, apy) = (-a.0, -a.1);
        let len2 = abx * abx + aby * aby;
        let t = if len2 > 0.0 {
            ((apx * abx + apy * aby) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let xt = (a.0 + t * abx).hypot(a.1 + t * aby);
        if xt < best_xt {
            best_xt = xt;
            best_leg = i;
            best_along = (t * abx).hypot(t * aby);
        }
    }
    let leg_len = gc_dist(
        anchors[best_leg].ll[0],
        anchors[best_leg].ll[1],
        anchors[best_leg + 1].ll[0],
        anchors[best_leg + 1].ll[1],
    );
    if best_along >= leg_len - 3.0 {
        best_leg + 1
    } else {
        best_leg
    }
}

/// Trim anchors already behind an airborne aircraft; prepend its current position (unnamed).
fn remaining_anchors(anchors: &[NamedAnchor], lat: f64, lon: f64, hdg: f64) -> Vec<NamedAnchor> {
    if anchors.len() < 2 {
        return anchors.to_vec();
    }
    let mut idx = project_forward_index(anchors, lat, lon);
    // Skip fixes clearly behind the current heading.
    while idx < anchors.len() - 1 {
        let d = gc_dist(lat, lon, anchors[idx].ll[0], anchors[idx].ll[1]);
        let brg = bearing_deg([lat, lon], anchors[idx].ll);
        if d > 5.0 && angle_diff(hdg, brg) > 110.0 {
            idx += 1;
        } else {
            break;
        }
    }
    let mut out = vec![NamedAnchor {
        name: String::new(),
        ll: [lat, lon],
    }];
    out.extend_from_slice(&anchors[idx..]);
    out
}

/// Trim the full filed route to the forward remainder using only along-route position, never
/// heading — a ground aircraft's heading is unreliable (parked, spun around on the ramp).
///
/// When `project_forward_index` lands on an *endpoint* of the whole route (index 0, or the last
/// index), that anchor IS the airport the ground aircraft currently occupies — the departure field
/// pre-push, or the arrival field once landed — so it's replaced by the aircraft's actual position
/// instead of kept as a separate point ahead of it: a pre-departure aircraft keeps every real
/// waypoint after the departure airport (still the whole future route); a landed aircraft has
/// nothing left after the arrival airport, so the path collapses to one point and `route_path`
/// reports no path at all. Any other index is a genuine, distinct waypoint still ahead, so the
/// current position is prepended in front of it, unchanged.
///
/// This is index-based, not a distance/epsilon match on the aircraft's coordinates — a real gate or
/// ramp position is essentially never the airport's exact reference point, so a naive "prepend
/// always" would still add a spurious extra point at the route's start and inflate
/// `predict::arrival_eta`'s "the nav engine resolved real waypoints" signal (`path.len() > 2`) for
/// ordinary pre-departure traffic on an otherwise-unresolved route.
fn forward_route_from_position(anchors: &[NamedAnchor], lat: f64, lon: f64) -> Vec<NamedAnchor> {
    if anchors.len() < 2 {
        return anchors.to_vec();
    }
    let idx = project_forward_index(anchors, lat, lon);
    let keep_from = if idx == 0 {
        1
    } else if idx == anchors.len() - 1 {
        anchors.len()
    } else {
        idx
    };
    let mut out = vec![NamedAnchor {
        name: String::new(),
        ll: [lat, lon],
    }];
    out.extend_from_slice(&anchors[keep_from..]);
    out
}

/// The first (nearest-along-route) crossing of `path` with the FCA polyline.
fn path_crossing(path: &[[f64; 2]], fca: &[[f64; 2]]) -> Option<FcaCrossing> {
    if path.len() < 2 || fca.len() < 2 {
        return None;
    }
    let refp = path[0];
    let mut cum = 0.0;
    let mut best: Option<FcaCrossing> = None;

    for i in 0..path.len() - 1 {
        let sub = densify(path[i], path[i + 1]);
        let mut sub_cum = cum;
        for s in 0..sub.len() - 1 {
            let seg_len = gc_dist(sub[s][0], sub[s][1], sub[s + 1][0], sub[s + 1][1]);
            let p1 = local(refp, sub[s]);
            let p2 = local(refp, sub[s + 1]);
            for j in 0..fca.len() - 1 {
                let c = local(refp, fca[j]);
                let d = local(refp, fca[j + 1]);
                if let Some((t, (ix, iy))) = seg_intersect(p1, p2, c, d) {
                    let along = sub_cum + t * seg_len;
                    if best.as_ref().is_none_or(|b| along < b.along_nm) {
                        let (clat, clon) = unlocal(refp, ix, iy);
                        best = Some(FcaCrossing {
                            lat: clat,
                            lon: clon,
                            along_nm: along,
                        });
                    }
                }
            }
            sub_cum += seg_len;
        }
        cum += gc_dist(path[i][0], path[i][1], path[i + 1][0], path[i + 1][1]);
    }
    best
}

/// Resolve an aircraft's filed route to a great-circle path (the remaining route, for
/// airborne aircraft). Returns None when the route can't be resolved to ≥2 anchors.
/// Resolve this ONCE per aircraft, then test it against many FCAs with `crosses`.
#[allow(clippy::too_many_arguments)]
pub fn route_path(
    nav: &NavData,
    airports: &AirportDb,
    dep: &str,
    arr: &str,
    route: &str,
    lat: f64,
    lon: f64,
    hdg: i64,
    gs: i64,
) -> Option<Vec<[f64; 2]>> {
    let dep = dep.to_ascii_uppercase();
    let arr = arr.to_ascii_uppercase();
    let anchors = route_anchors(nav, airports, &dep, &arr, route);
    if anchors.len() < 2 {
        return None;
    }
    let path = if gs >= 50 {
        remaining_anchors(&anchors, lat, lon, hdg as f64)
    } else {
        forward_route_from_position(&anchors, lat, lon)
    };
    let path: Vec<[f64; 2]> = path.into_iter().map(|a| a.ll).collect();
    (path.len() >= 2).then_some(path)
}

/// Same resolution as [`route_path`], but keeps each named fix's identifier and its cumulative
/// along-route distance (nm) instead of collapsing to a bare polyline. Distance accumulates
/// across *every* anchor (named and unnamed) so an unnamed procedure-leg point in between doesn't
/// undercount the distance to the next named fix. Shares `route_path`'s exact
/// airborne/ground branch (`remaining_anchors` / `forward_route_from_position`), so the fixes and
/// distances reported here are the same ones the real ETA model (via `route_path`) is measuring
/// against — not a separate, potentially-diverging resolution.
#[allow(clippy::too_many_arguments)]
pub fn route_path_named(
    nav: &NavData,
    airports: &AirportDb,
    dep: &str,
    arr: &str,
    route: &str,
    lat: f64,
    lon: f64,
    hdg: i64,
    gs: i64,
) -> Option<Vec<(String, f64, f64, f64)>> {
    let dep = dep.to_ascii_uppercase();
    let arr = arr.to_ascii_uppercase();
    let anchors = route_anchors(nav, airports, &dep, &arr, route);
    if anchors.len() < 2 {
        return None;
    }
    let path = if gs >= 50 {
        remaining_anchors(&anchors, lat, lon, hdg as f64)
    } else {
        forward_route_from_position(&anchors, lat, lon)
    };
    if path.len() < 2 {
        return None;
    }

    let mut cum = 0.0;
    let mut out = Vec::new();
    for i in 0..path.len() {
        if i > 0 {
            cum += gc_dist(
                path[i - 1].ll[0],
                path[i - 1].ll[1],
                path[i].ll[0],
                path[i].ll[1],
            );
        }
        if !path[i].name.is_empty() {
            out.push((path[i].name.clone(), path[i].ll[0], path[i].ll[1], cum));
        }
    }
    Some(out)
}

/// Where a pre-resolved `path` crosses the FCA line. For airborne aircraft the `path` is already the
/// *remaining* route (from the current position forward, via [`route_path`]/`remaining_anchors`), so
/// any crossing it contains is genuinely ahead on the route. We deliberately do NOT additionally gate
/// on current heading: matching vatflow's `validRouteCrossing`, a straight-line "is the crossing point
/// ahead of my nose" test drops aircraft on holds/vectors/outbound dogleg legs whose route still
/// crosses (e.g. filed `… EMI299018 … KOZAR …` that heads east before turning back through the FCA).
pub fn crosses(path: &[[f64; 2]], fca_points: &[[f64; 2]]) -> Option<FcaCrossing> {
    path_crossing(path, fca_points)
}

/// Whether an aircraft's filed route crosses the FCA, and where.
#[allow(clippy::too_many_arguments)]
pub fn crossing_for(
    fca_points: &[[f64; 2]],
    nav: &NavData,
    airports: &AirportDb,
    dep: &str,
    arr: &str,
    route: &str,
    lat: f64,
    lon: f64,
    hdg: i64,
    gs: i64,
) -> Option<FcaCrossing> {
    let path = route_path(nav, airports, dep, arr, route, lat, lon, hdg, gs)?;
    crosses(&path, fca_points)
}

// --- metering (sequence crossing traffic) ---

/// One aircraft to sequence across the FCA.
pub struct MeterInput {
    /// Unmetered ETA to the crossing, epoch millis.
    pub eta_ms: i64,
    /// Airborne aircraft sort ahead of advisory ground (they keep priority), but are still separated
    /// from one another — an airborne crossing is pushed later only when it would conflict with an
    /// earlier committed crossing, so two aircraft never share a slot.
    pub airborne: bool,
    /// Predicted crossing groundspeed (kt) — used for MIT spacing.
    pub cross_speed: f64,
    /// A frozen (issued-CFR) metered crossing time; pins this aircraft like an airborne one.
    pub frozen_ms: Option<i64>,
}

pub struct MeterOutput {
    /// Metered crossing time, epoch millis.
    pub sched_ms: i64,
    pub delay_sec: i64,
    /// 1-based order by metered time.
    pub seq: i64,
}

/// Sequence crossing traffic. Auto mode: airborne + frozen (issued-CFR) aircraft are
/// fixed constraints; unreleased ground floats into the first gap clear of every
/// committed crossing by the separation (rate → constant MINIT; MIT → distance ÷ cross
/// speed). Manual mode (`order` = candidate indices): chain in the controller's order,
/// spacing each behind the previous, splicing any newcomers by ETA. Ported from
/// vatflow's `scheduleAuto` / `scheduleCandidates`.
pub fn meter(
    cands: &[MeterInput],
    mode: &str,
    rate: i32,
    mit: i32,
    order: Option<&[usize]>,
) -> Vec<MeterOutput> {
    let sep_ms = |c: &MeterInput| -> i64 {
        let secs = if mode == "mit" {
            (mit as f64 / c.cross_speed.max(60.0)) * 3600.0
        } else if rate > 0 {
            3600.0 / rate as f64
        } else {
            0.0
        };
        (secs * 1000.0) as i64
    };

    let n = cands.len();
    let mut sched = vec![0i64; n];
    let mut seq_of = vec![0i64; n];

    if let Some(seq) = order {
        // Manual: honour the controller's order; splice newcomers by ETA at the end.
        let mut ordered: Vec<usize> = seq.iter().copied().filter(|&i| i < n).collect();
        let seen: std::collections::HashSet<usize> = ordered.iter().copied().collect();
        let mut rest: Vec<usize> = (0..n).filter(|i| !seen.contains(i)).collect();
        rest.sort_by_key(|&i| cands[i].eta_ms);
        ordered.extend(rest);
        let mut prev: Option<i64> = None;
        for &i in &ordered {
            let c = &cands[i];
            // In manual mode the controller's order is authoritative, so every crossing — airborne
            // included — is chained behind the previous one by the required separation. Moving an
            // aircraft up therefore pushes the ones now behind it later: the delay a controller would
            // have to create with vectors/speed control. An issued CFR is the only hard, un-moveable
            // commitment; it keeps its frozen time (and still spaces those chained behind it).
            sched[i] = if let Some(f) = c.frozen_ms {
                f
            } else {
                match prev {
                    Some(p) => c.eta_ms.max(p + sep_ms(c)),
                    None => c.eta_ms,
                }
            };
            prev = Some(sched[i]);
        }
        // Manual: the crossing sequence IS the controller's order — assigned directly, not derived
        // from scheduled time. That keeps a dragged aircraft in the slot it was moved to even though
        // airborne/frozen crossings keep their true (un-delayed) times, which would otherwise re-sort
        // it away under a by-time ranking.
        for (rank, &i) in ordered.iter().enumerate() {
            seq_of[i] = rank as i64 + 1;
        }
    } else {
        // Auto: pinned (airborne + issued CFR) sort ahead of advisory ground so they keep priority;
        // then every crossing is separated in that order. Each aircraft takes the earliest slot at or
        // after its ETA that is clear of the already-committed crossings — so it's pushed later ONLY
        // when it would actually conflict (two well-spaced aircraft both stay "on time"). An issued
        // CFR keeps its frozen time (it was spaced from the others when it was released).
        let pinned = |c: &MeterInput| c.airborne || c.frozen_ms.is_some();
        let mut o: Vec<usize> = (0..n).collect();
        o.sort_by(|&a, &b| {
            pinned(&cands[b])
                .cmp(&pinned(&cands[a]))
                .then(cands[a].eta_ms.cmp(&cands[b].eta_ms))
        });
        let mut committed: Vec<i64> = Vec::new();
        for &i in &o {
            let c = &cands[i];
            sched[i] = if let Some(f) = c.frozen_ms {
                f
            } else {
                // Airborne and ground alike: no two crossings may share a slot.
                earliest_slot(c.eta_ms, &committed, sep_ms(c))
            };
            committed.push(sched[i]);
        }
        // Auto: sequence by scheduled crossing time.
        let mut by_time: Vec<usize> = (0..n).collect();
        by_time.sort_by_key(|&i| sched[i]);
        for (rank, &i) in by_time.iter().enumerate() {
            seq_of[i] = rank as i64 + 1;
        }
    }

    (0..cands.len())
        .map(|i| MeterOutput {
            sched_ms: sched[i],
            delay_sec: ((sched[i] - cands[i].eta_ms).max(0)) / 1000,
            seq: seq_of[i],
        })
        .collect()
}

/// Earliest time ≥ `eta` clear of every committed crossing by `sep` (ms).
pub fn earliest_slot(eta: i64, committed: &[i64], sep: i64) -> i64 {
    if sep <= 0 {
        return eta;
    }
    let mut sorted = committed.to_vec();
    sorted.sort_unstable();
    let mut t = eta;
    loop {
        let mut bumped = false;
        for &c in &sorted {
            if (t - c).abs() < sep {
                t = c + sep;
                bumped = true;
            }
        }
        if !bumped {
            break;
        }
    }
    t
}

// --- geometry primitives ---

fn local(refp: [f64; 2], p: [f64; 2]) -> (f64, f64) {
    let x = (p[1] - refp[1]) * NM_PER_DEG * refp[0].to_radians().cos();
    let y = (p[0] - refp[0]) * NM_PER_DEG;
    (x, y)
}

fn unlocal(refp: [f64; 2], x: f64, y: f64) -> (f64, f64) {
    let lat = refp[0] + y / NM_PER_DEG;
    let lon = refp[1] + x / (NM_PER_DEG * refp[0].to_radians().cos());
    (lat, lon)
}

/// 2D segment intersection; returns (parameter along p1→p2, intersection point).
fn seg_intersect(
    p1: (f64, f64),
    p2: (f64, f64),
    p3: (f64, f64),
    p4: (f64, f64),
) -> Option<(f64, (f64, f64))> {
    let r = (p2.0 - p1.0, p2.1 - p1.1);
    let s = (p4.0 - p3.0, p4.1 - p3.1);
    let denom = r.0 * s.1 - r.1 * s.0;
    if denom.abs() < 1e-9 {
        return None;
    }
    let qp = (p3.0 - p1.0, p3.1 - p1.1);
    let t = (qp.0 * s.1 - qp.1 * s.0) / denom;
    let u = (qp.0 * r.1 - qp.1 * r.0) / denom;
    if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
        Some((t, (p1.0 + t * r.0, p1.1 + t * r.1)))
    } else {
        None
    }
}

/// Great-circle-interpolated points along a leg, ~`DENSIFY_STEP_NM` apart.
fn densify(a: [f64; 2], b: [f64; 2]) -> Vec<[f64; 2]> {
    let dist = gc_dist(a[0], a[1], b[0], b[1]);
    let n = ((dist / DENSIFY_STEP_NM).ceil() as usize).max(1);
    if n == 1 {
        return vec![a, b];
    }
    (0..=n).map(|k| slerp(a, b, k as f64 / n as f64)).collect()
}

pub(crate) fn slerp(a: [f64; 2], b: [f64; 2], f: f64) -> [f64; 2] {
    let (lat1, lon1) = (a[0].to_radians(), a[1].to_radians());
    let (lat2, lon2) = (b[0].to_radians(), b[1].to_radians());
    let d = 2.0
        * (((lat1 - lat2) / 2.0).sin().powi(2)
            + lat1.cos() * lat2.cos() * ((lon1 - lon2) / 2.0).sin().powi(2))
        .sqrt()
        .asin();
    if d < 1e-9 {
        return a;
    }
    let aa = ((1.0 - f) * d).sin() / d.sin();
    let bb = (f * d).sin() / d.sin();
    let x = aa * lat1.cos() * lon1.cos() + bb * lat2.cos() * lon2.cos();
    let y = aa * lat1.cos() * lon1.sin() + bb * lat2.cos() * lon2.sin();
    let z = aa * lat1.sin() + bb * lat2.sin();
    let lat = z.atan2((x * x + y * y).sqrt());
    let lon = y.atan2(x);
    [lat.to_degrees(), lon.to_degrees()]
}

pub(crate) fn bearing_deg(a: [f64; 2], b: [f64; 2]) -> f64 {
    let (lat1, lat2) = (a[0].to_radians(), b[0].to_radians());
    let dlon = (b[1] - a[1]).to_radians();
    let y = dlon.sin() * lat2.cos();
    let x = lat1.cos() * lat2.sin() - lat1.sin() * lat2.cos() * dlon.cos();
    (y.atan2(x).to_degrees() + 360.0) % 360.0
}

fn angle_diff(a: f64, b: f64) -> f64 {
    let d = (a - b).abs() % 360.0;
    if d > 180.0 { 360.0 - d } else { d }
}

/// Position and heading `target_nm` along `path` (a `[lat, lon]` polyline from its start), for
/// #226's forward prediction scrubber — the geometric counterpart to
/// `predict::project_along_route`'s along-route distance. Clamps to the last point (with that
/// leg's bearing) once `target_nm` reaches or exceeds the polyline's total length. `path` must have
/// at least 2 points — the caller (only ever fed an already-resolved [`route_path`]) guarantees
/// this.
pub(crate) fn point_and_heading_at(path: &[[f64; 2]], target_nm: f64) -> ([f64; 2], f64) {
    let mut acc = 0.0;
    for w in path.windows(2) {
        let seg = gc_dist(w[0][0], w[0][1], w[1][0], w[1][1]);
        if target_nm <= acc + seg {
            let f = if seg > 0.0 {
                ((target_nm - acc) / seg).clamp(0.0, 1.0)
            } else {
                0.0
            };
            return (slerp(w[0], w[1], f), bearing_deg(w[0], w[1]));
        }
        acc += seg;
    }
    let last = path.len() - 1;
    (path[last], bearing_deg(path[last - 1], path[last]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::airports::Airport;
    use std::collections::HashMap;

    fn airports() -> AirportDb {
        // KJFK (New York) and KIAD (Washington Dulles), roughly.
        HashMap::from([
            ("KJFK".to_string(), Airport::at(40.64, -73.78)),
            ("KIAD".to_string(), Airport::at(38.95, -77.46)),
        ])
    }

    #[test]
    fn detects_a_route_crossing_a_line() {
        let nav = NavData::default(); // no fixes; dep→arr great circle
        let ap = airports();
        // A near-vertical FCA line around lon -75.7, spanning the JFK→IAD track latitudes.
        let fca = [[41.0, -75.7], [38.0, -75.7]];
        let c = crossing_for(&fca, &nav, &ap, "KJFK", "KIAD", "", 0.0, 0.0, 0, 0);
        assert!(c.is_some(), "JFK→IAD should cross a line at lon -75.7");
        let c = c.unwrap();
        assert!(
            c.lon > -76.2 && c.lon < -75.2,
            "crossing lon ~ -75.7, got {}",
            c.lon
        );
        assert!(c.along_nm > 0.0);
    }

    #[test]
    fn no_crossing_when_line_is_off_route() {
        let nav = NavData::default();
        let ap = airports();
        // A line far west of the JFK→IAD track.
        let fca = [[41.0, -90.0], [38.0, -90.0]];
        assert!(crossing_for(&fca, &nav, &ap, "KJFK", "KIAD", "", 0.0, 0.0, 0, 0).is_none());
    }

    #[test]
    fn ground_at_destination_does_not_match_an_already_passed_fca() {
        // Regression (#213): a JFK→IAD flight that has landed and is on the ground at KIAD must not
        // still show as crossing the lon -75.7 line it flew through en route — the same geometry
        // `detects_a_route_crossing_a_line` confirms the full route crosses.
        let nav = NavData::default();
        let ap = airports();
        let fca = [[41.0, -75.7], [38.0, -75.7]];
        let c = crossing_for(&fca, &nav, &ap, "KJFK", "KIAD", "", 38.95, -77.46, 0, 5);
        assert!(
            c.is_none(),
            "a ground aircraft at its destination shouldn't match an FCA it already passed"
        );
    }

    #[test]
    fn ground_at_origin_still_matches_a_future_fca() {
        // A pre-departure aircraft still on the ground at KJFK must still show its future crossing —
        // trimming ground routes for #213 must not also drop a flight that hasn't left yet.
        let nav = NavData::default();
        let ap = airports();
        let fca = [[41.0, -75.7], [38.0, -75.7]];
        let c = crossing_for(&fca, &nav, &ap, "KJFK", "KIAD", "", 40.64, -73.78, 0, 0);
        assert!(
            c.is_some(),
            "a pre-departure aircraft at its origin should still match a future crossing"
        );
    }

    #[test]
    fn ground_at_origin_on_the_reverse_leg_still_matches_a_future_fca() {
        // Regression: `project_forward_index` is a plain geometric nearest-leg projection with no
        // knowledge of "this position is a placeholder" — the caller must pass a real position.
        // A prefile with no live position used to pass (0.0, 0.0) here, which happens to project
        // onto the *arrival* end for this reversed KIAD->KJFK route (the same fixture the other
        // tests use, direction swapped), collapsing the route to one point and losing a real future
        // crossing entirely. With the caller now passing the real departure airport's coordinates
        // (`handlers::flow::prefile_position`), it must still resolve correctly here too.
        let nav = NavData::default();
        let ap = airports();
        let fca = [[41.0, -75.7], [38.0, -75.7]];
        let c = crossing_for(&fca, &nav, &ap, "KIAD", "KJFK", "", 38.95, -77.46, 0, 0);
        assert!(
            c.is_some(),
            "a pre-departure aircraft at KIAD (departing to KJFK) should still match a future crossing"
        );
    }

    #[test]
    fn loads_bundled_nav_and_matches_a_real_route() {
        let nav = NavData::load();
        assert!(
            nav.len() > 60_000,
            "nav db should load tens of thousands of points"
        );
        let empty = HashMap::new();
        // Sea Isle VOR (SIE) ~ 39.10, -74.80.
        let sie = nav.resolve("SIE", &empty, None).expect("SIE resolves");
        assert!((sie[0] - 39.10).abs() < 0.3 && (sie[1] + 74.80).abs() < 0.3);

        // A JFK→DCA route down the NJ coast via real fixes; a line at lat 39.5 across
        // the corridor sits between WHITE (40.0) and SIE (39.1), so it must be crossed.
        let ap = HashMap::from([
            ("KJFK".to_string(), Airport::at(40.64, -73.78)),
            ("KDCA".to_string(), Airport::at(38.85, -77.04)),
        ]);
        let fca = [[39.5, -75.6], [39.5, -74.0]];
        let c = crossing_for(
            &fca,
            &nav,
            &ap,
            "KJFK",
            "KDCA",
            "RBV WHITE SIE",
            0.0,
            0.0,
            0,
            0,
        );
        assert!(
            c.is_some(),
            "route via RBV WHITE SIE should cross the lat-39.5 line"
        );
        assert!((c.unwrap().lat - 39.5).abs() < 0.2);
    }

    #[test]
    fn route_path_named_accumulates_distance_from_departure_when_on_the_ground() {
        let nav = NavData::load();
        let ap = HashMap::from([
            ("KJFK".to_string(), Airport::at(40.64, -73.78)),
            ("KDCA".to_string(), Airport::at(38.85, -77.04)),
        ]);
        // gs=0 takes the ground branch, positioned at the departure airport (as a prefile / parked
        // aircraft is) — trimming keeps the whole future route, measured from the field.
        let fixes = route_path_named(
            &nav,
            &ap,
            "KJFK",
            "KDCA",
            "RBV WHITE SIE",
            40.64,
            -73.78,
            0,
            0,
        )
        .expect("route should resolve");
        let names: Vec<&str> = fixes.iter().map(|(n, ..)| n.as_str()).collect();
        assert!(
            names.contains(&"RBV") && names.contains(&"WHITE") && names.contains(&"SIE"),
            "expected RBV/WHITE/SIE among named fixes, got {names:?}"
        );
        // Distance is cumulative and non-decreasing, starting near 0 at the first named fix.
        assert!(fixes[0].3 >= 0.0);
        for w in fixes.windows(2) {
            assert!(
                w[1].3 >= w[0].3,
                "distance should never decrease: {} then {}",
                w[0].3,
                w[1].3
            );
        }
        // SIE is further from JFK than RBV (down the coast toward DCA).
        let rbv_d = fixes.iter().find(|(n, ..)| n == "RBV").unwrap().3;
        let sie_d = fixes.iter().find(|(n, ..)| n == "SIE").unwrap().3;
        assert!(
            sie_d > rbv_d,
            "SIE ({sie_d}) should be farther than RBV ({rbv_d})"
        );
        // Magnitude, not just ordering: the total matches `route_path`'s length for the same
        // input (a phantom leading point would add thousands of nm and still be monotonic).
        let raw_path = route_path(
            &nav,
            &ap,
            "KJFK",
            "KDCA",
            "RBV WHITE SIE",
            40.64,
            -73.78,
            0,
            0,
        )
        .expect("route_path resolves the same route");
        let total = crate::feed::predict::path_len_nm(&raw_path);
        let last_d = fixes.last().unwrap().3;
        assert!(
            (last_d - total).abs() < 0.5 && total < 300.0,
            "total {last_d} nm should match route_path's {total} nm (~224 nm KJFK→KDCA)"
        );
    }

    #[test]
    fn route_path_named_trims_a_ground_aircraft_at_a_real_mid_route_position() {
        // A ground aircraft (gs < 50) with a *real* current position partway down the route must
        // get the same forward-trimmed remainder `route_path` gives the real ETA model — not the
        // full untrimmed filed route from departure. Reusing `forward_route_from_position` (shared
        // with `route_path`) is what makes that hold; a version that fell back to the bare anchor
        // list here would report RBV as still ahead and its total distance would diverge from
        // `route_path`'s (`path_len_nm`) — the exact number `predict::arrival_eta` times the real
        // ETA against.
        let nav = NavData::load();
        let ap = HashMap::from([
            ("KJFK".to_string(), Airport::at(40.64, -73.78)),
            ("KDCA".to_string(), Airport::at(38.85, -77.04)),
        ]);
        let empty = HashMap::new();
        let white = nav.resolve("WHITE", &empty, None).expect("WHITE resolves");
        let sie = nav.resolve("SIE", &empty, None).expect("SIE resolves");
        // Same real position as the airborne test, but gs=0 — a taxiing/rolling ground aircraft,
        // not an unresolved-position prefile placeholder (which the other ground test covers).
        let pos = [white[0] * 0.2 + sie[0] * 0.8, white[1] * 0.2 + sie[1] * 0.8];
        let fixes = route_path_named(
            &nav,
            &ap,
            "KJFK",
            "KDCA",
            "RBV WHITE SIE",
            pos[0],
            pos[1],
            0,
            0,
        )
        .expect("remaining route should resolve");
        let names: Vec<&str> = fixes.iter().map(|(n, ..)| n.as_str()).collect();
        assert!(
            !names.contains(&"RBV"),
            "a ground aircraft already well past RBV must not still show it ahead, got {names:?}"
        );
        assert!(
            names.contains(&"SIE"),
            "SIE should still be ahead, got {names:?}"
        );
        // The last fix's cumulative distance must match route_path's own total length for the
        // identical input — not necessarily the short "as the crow flies from here" distance,
        // since ground trimming is index-based (keeps the current leg's start point) and so can
        // legitimately retrace a short stretch, same as `route_path`'s raw polyline does.
        let raw_path = route_path(
            &nav,
            &ap,
            "KJFK",
            "KDCA",
            "RBV WHITE SIE",
            pos[0],
            pos[1],
            0,
            0,
        )
        .expect("route_path should resolve the same remainder");
        let expected_total = crate::feed::predict::path_len_nm(&raw_path);
        let last_d = fixes.last().unwrap().3;
        assert!(
            (last_d - expected_total).abs() < 0.5,
            "route_path_named's total distance ({last_d}) should match route_path's \
             path_len_nm ({expected_total}) for the identical input"
        );
    }

    #[test]
    fn route_path_named_measures_from_current_position_when_airborne() {
        let nav = NavData::load();
        let ap = HashMap::from([
            ("KJFK".to_string(), Airport::at(40.64, -73.78)),
            ("KDCA".to_string(), Airport::at(38.85, -77.04)),
        ]);
        let empty = HashMap::new();
        let white = nav.resolve("WHITE", &empty, None).expect("WHITE resolves");
        let sie = nav.resolve("SIE", &empty, None).expect("SIE resolves");
        // Positioned just short of SIE (between WHITE and SIE), heading down the coast toward
        // it — gs=200 takes the airborne branch. Both WHITE and RBV (further back) should drop.
        let hdg = bearing_deg(white, sie).round() as i64;
        let pos = [white[0] * 0.2 + sie[0] * 0.8, white[1] * 0.2 + sie[1] * 0.8];
        let fixes = route_path_named(
            &nav,
            &ap,
            "KJFK",
            "KDCA",
            "RBV WHITE SIE",
            pos[0],
            pos[1],
            hdg,
            200,
        )
        .expect("remaining route should resolve");
        let names: Vec<&str> = fixes.iter().map(|(n, ..)| n.as_str()).collect();
        assert!(
            !names.contains(&"RBV") && !names.contains(&"WHITE"),
            "RBV/WHITE should be behind current position, got {names:?}"
        );
        assert!(
            names.contains(&"SIE"),
            "SIE should still be ahead, got {names:?}"
        );
        // Distance is measured from the current position (near SIE), not from JFK — small, not
        // the ~hundred-nm full-route figure the ground-branch test sees.
        let sie_d = fixes.iter().find(|(n, ..)| n == "SIE").unwrap().3;
        assert!(
            sie_d < 30.0,
            "expected SIE within ~30nm of current position, got {sie_d}"
        );
    }

    #[test]
    fn meters_ground_traffic_by_rate() {
        // 30/hr → 120s separation; three ground aircraft close together.
        let cands = vec![
            MeterInput {
                eta_ms: 0,
                airborne: false,
                cross_speed: 400.0,
                frozen_ms: None,
            },
            MeterInput {
                eta_ms: 60_000,
                airborne: false,
                cross_speed: 400.0,
                frozen_ms: None,
            },
            MeterInput {
                eta_ms: 200_000,
                airborne: false,
                cross_speed: 400.0,
                frozen_ms: None,
            },
        ];
        let out = meter(&cands, "rate", 30, 15, None);
        assert_eq!(out[0].sched_ms, 0);
        assert_eq!(out[1].sched_ms, 120_000); // bumped 120s after the first
        assert_eq!(out[2].sched_ms, 240_000); // bumped 120s after the second
        assert_eq!(out[1].delay_sec, 60);
        assert_eq!(out[2].delay_sec, 40);
        assert_eq!((out[0].seq, out[1].seq, out[2].seq), (1, 2, 3));
    }

    #[test]
    fn airborne_holds_priority_over_ground() {
        // Ground at ETA 0 must yield to an airborne aircraft crossing at 30s.
        let cands = vec![
            MeterInput {
                eta_ms: 0,
                airborne: false,
                cross_speed: 400.0,
                frozen_ms: None,
            },
            MeterInput {
                eta_ms: 30_000,
                airborne: true,
                cross_speed: 450.0,
                frozen_ms: None,
            },
        ];
        let out = meter(&cands, "rate", 30, 15, None);
        assert_eq!(out[1].sched_ms, 30_000); // airborne keeps its ETA
        assert_eq!(out[1].delay_sec, 0);
        assert!(
            out[0].sched_ms >= 150_000,
            "ground slots 120s after the airborne"
        );
        assert_eq!(out[1].seq, 1);
        assert_eq!(out[0].seq, 2);
    }

    #[test]
    fn frozen_release_is_pinned_and_ground_floats_around_it() {
        // A frozen (issued-CFR) crossing at 100s holds; an advisory ground at ETA 0
        // yields to the first slot ≥ 0 clear of it by the 120s separation.
        let cands = vec![
            MeterInput {
                eta_ms: 0,
                airborne: false,
                cross_speed: 400.0,
                frozen_ms: None,
            },
            MeterInput {
                eta_ms: 90_000,
                airborne: false,
                cross_speed: 400.0,
                frozen_ms: Some(100_000),
            },
        ];
        let out = meter(&cands, "rate", 30, 15, None);
        assert_eq!(out[1].sched_ms, 100_000); // frozen stays put
        assert_eq!(out[0].sched_ms, 220_000); // advisory floats 120s after it
    }

    #[test]
    fn auto_separates_conflicting_airborne_crossings() {
        // Two airborne aircraft would cross within the separation → the later one is pushed back so
        // they never share a slot (the real bug: pinned crossings weren't separated). A third, well
        // clear, stays on time.
        let cands = vec![
            MeterInput {
                eta_ms: 0,
                airborne: true,
                cross_speed: 400.0,
                frozen_ms: None,
            },
            MeterInput {
                eta_ms: 30_000, // within the 120s separation of #0
                airborne: true,
                cross_speed: 400.0,
                frozen_ms: None,
            },
            MeterInput {
                eta_ms: 400_000, // well clear
                airborne: true,
                cross_speed: 400.0,
                frozen_ms: None,
            },
        ];
        let out = meter(&cands, "rate", 30, 15, None); // 120s separation
        assert_eq!(out[0].sched_ms, 0);
        assert_eq!(
            out[1].sched_ms, 120_000,
            "second airborne bumped clear of the first"
        );
        assert_eq!(out[1].delay_sec, 90); // 120_000 − 30_000
        assert_ne!(
            out[0].sched_ms, out[1].sched_ms,
            "no two crossings share a slot"
        );
        assert_eq!(
            out[2].sched_ms, 400_000,
            "well-clear airborne stays on time"
        );
        assert_eq!(out[2].delay_sec, 0);
    }

    #[test]
    fn manual_order_chains_airborne_and_pins_frozen() {
        // In manual mode the controller's order is authoritative: airborne crossings ARE chained
        // (they can be shown as needing delay), while an issued CFR keeps its frozen time.
        let cands = vec![
            MeterInput {
                eta_ms: 0,
                airborne: false,
                cross_speed: 400.0,
                frozen_ms: None,
            },
            MeterInput {
                eta_ms: 30_000,
                airborne: true,
                cross_speed: 450.0,
                frozen_ms: None,
            },
            MeterInput {
                eta_ms: 40_000,
                airborne: false,
                cross_speed: 400.0,
                frozen_ms: Some(50_000),
            },
        ];
        // rate 30 → 120s separation.
        let out = meter(&cands, "rate", 30, 15, Some(&[0, 1, 2]));
        assert_eq!(out[0].sched_ms, 0); // first keeps its ETA
        // Airborne is chained behind #0 by 120s and thus delayed (this is the reorder behavior).
        assert_eq!(
            out[1].sched_ms, 120_000,
            "airborne chains behind the previous crossing"
        );
        assert_eq!(out[1].delay_sec, 90); // 120_000 − 30_000
        assert_eq!(out[2].sched_ms, 50_000, "frozen CFR must stay pinned");
        // Sequence follows the controller's order, not the times.
        assert_eq!((out[0].seq, out[1].seq, out[2].seq), (1, 2, 3));
    }

    #[test]
    fn reorder_up_delays_everyone_behind_the_new_slot() {
        // Mirrors the vatflow case: a far aircraft (D, ETA 900s) dragged ahead of three nearer
        // ones pushes those three behind it, each chained by the 120s separation.
        let mk = |eta: i64| MeterInput {
            eta_ms: eta,
            airborne: true,
            cross_speed: 400.0,
            frozen_ms: None,
        };
        // Natural ETAs: A=200 B=400 C=600 D=900 (all airborne).
        let cands = vec![mk(200_000), mk(400_000), mk(600_000), mk(900_000)];
        // Controller drags D to the front: order [D, A, B, C] = indices [3, 0, 1, 2].
        let out = meter(&cands, "rate", 30, 15, Some(&[3, 0, 1, 2]));
        assert_eq!(
            out[3].sched_ms, 900_000,
            "the moved aircraft keeps its own ETA"
        );
        assert_eq!(out[3].delay_sec, 0);
        // A/B/C are now chained behind D at 120s spacing, so all are delayed.
        assert_eq!(out[0].sched_ms, 1_020_000); // 900k + 120k
        assert_eq!(out[1].sched_ms, 1_140_000);
        assert_eq!(out[2].sched_ms, 1_260_000);
        assert!(out[0].delay_sec > 0 && out[1].delay_sec > 0 && out[2].delay_sec > 0);
        assert_eq!(
            (out[3].seq, out[0].seq, out[1].seq, out[2].seq),
            (1, 2, 3, 4)
        );
    }

    #[test]
    fn manual_order_chains_by_separation() {
        // Three aircraft all ETA 0; controller's order [2,1,0] chains them 120s apart.
        let cands = (0..3)
            .map(|_| MeterInput {
                eta_ms: 0,
                airborne: false,
                cross_speed: 400.0,
                frozen_ms: None,
            })
            .collect::<Vec<_>>();
        let out = meter(&cands, "rate", 30, 15, Some(&[2, 1, 0]));
        assert_eq!(out[2].sched_ms, 0);
        assert_eq!(out[1].sched_ms, 120_000);
        assert_eq!(out[0].sched_ms, 240_000);
        assert_eq!((out[2].seq, out[1].seq, out[0].seq), (1, 2, 3));
    }

    #[test]
    fn manual_order_seq_follows_controller_not_eta() {
        // Regression: the controller drags a ground aircraft (later ETA) ahead of an airborne one
        // (earlier ETA). A by-time ranking would snap the airborne crossing back to seq 1; the
        // manual order must win so the dragged aircraft keeps the slot it was moved to. The airborne
        // aircraft, now sequenced behind, is chained (delayed) rather than crossing at its raw ETA.
        let cands = vec![
            MeterInput {
                eta_ms: 100_000, // ground, later
                airborne: false,
                cross_speed: 400.0,
                frozen_ms: None,
            },
            MeterInput {
                eta_ms: 50_000, // airborne, earlier
                airborne: true,
                cross_speed: 450.0,
                frozen_ms: None,
            },
        ];
        let out = meter(&cands, "rate", 30, 15, Some(&[0, 1]));
        assert_eq!(out[0].seq, 1, "the dragged ground aircraft keeps seq 1");
        assert_eq!(
            out[1].seq, 2,
            "the airborne aircraft stays where it was dragged"
        );
        assert_eq!(
            out[1].sched_ms, 220_000,
            "airborne is chained 120s behind the ground crossing (100k), so it's delayed"
        );
        assert!(out[1].delay_sec > 0);
    }

    #[test]
    fn airborne_past_the_line_does_not_match() {
        let nav = NavData::default();
        let ap = airports();
        let fca = [[41.0, -75.7], [38.0, -75.7]];
        // Aircraft already west of the line, heading further west (270°). Its remaining route runs
        // to KIAD (further west still) and never returns to the line, so there's no crossing ahead.
        let c = crossing_for(&fca, &nav, &ap, "KJFK", "KIAD", "", 39.2, -76.5, 270, 400);
        assert!(c.is_none(), "an aircraft past the line shouldn't match");
    }

    #[test]
    fn dogleg_route_crosses_even_when_heading_away() {
        // Regression: an aircraft flying AWAY from an FCA whose route still crosses it must match.
        // The jet is at (39.0, -76.0) tracking east to a fix at -75.0, then doglegs back northwest
        // through a line at lon -77.0 (mirrors a `… EMI299018 … KOZAR …` reroute). The straight-line
        // bearing from its nose to the crossing is ~northwest (~150° off its easterly heading), which
        // the old heading gate wrongly rejected. The remaining route clearly crosses, so it matches.
        let fca = [[40.0, -77.0], [38.0, -77.0]];
        let path = [[39.0, -76.0], [39.0, -75.0], [39.5, -78.0]];
        let c = crosses(&path, &fca).expect("dogleg route crosses the lon -77.0 line");
        assert!(
            c.lon > -77.3 && c.lon < -76.7,
            "crossing lon ~ -77.0, got {}",
            c.lon
        );
    }

    // ---- point_and_heading_at: for #226's forward prediction scrubber ----

    fn path_len(path: &[[f64; 2]]) -> f64 {
        path.windows(2)
            .map(|w| gc_dist(w[0][0], w[0][1], w[1][0], w[1][1]))
            .sum()
    }

    #[test]
    fn point_and_heading_at_returns_the_exact_endpoints() {
        let path = [[40.0, -74.0], [39.0, -75.0], [38.0, -76.0]];
        let (start, _) = point_and_heading_at(&path, 0.0);
        assert_eq!(start, path[0]);

        let total = path_len(&path);
        let (end, _) = point_and_heading_at(&path, total);
        assert!(
            gc_dist(end[0], end[1], path[2][0], path[2][1]) < 0.1,
            "expected the last point, got {end:?}"
        );
    }

    #[test]
    fn point_and_heading_at_interpolates_within_the_bracketing_leg() {
        let path = [[40.0, -74.0], [39.0, -75.0], [38.0, -76.0]];
        let leg1 = gc_dist(path[0][0], path[0][1], path[1][0], path[1][1]);
        // Halfway into the first leg should land roughly on the great-circle midpoint, not on
        // either endpoint or spilling into the second leg.
        let (mid, heading) = point_and_heading_at(&path, leg1 / 2.0);
        let d_from_start = gc_dist(path[0][0], path[0][1], mid[0], mid[1]);
        assert!(
            (d_from_start - leg1 / 2.0).abs() < 1.0,
            "expected ~{}nm from the start, got {d_from_start}nm",
            leg1 / 2.0
        );
        let expected_heading = bearing_deg(path[0], path[1]);
        assert!(
            angle_diff(heading, expected_heading) < 1.0,
            "heading {heading} should match the first leg's bearing {expected_heading}"
        );
    }

    #[test]
    fn point_and_heading_at_clamps_past_the_end() {
        let path = [[40.0, -74.0], [39.0, -75.0]];
        let total = path_len(&path);
        let (past, _) = point_and_heading_at(&path, total + 500.0);
        assert!(
            gc_dist(past[0], past[1], path[1][0], path[1][1]) < 0.1,
            "overshoot must clamp to the last point, got {past:?}"
        );
    }
}
