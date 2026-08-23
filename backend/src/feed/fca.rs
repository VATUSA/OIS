//! FCA geometry — resolve a filed route to a great-circle path and test whether it
//! crosses an FCA polyline. Ported from vatflow's `fca-metering.js` / `route-engine.js`
//! crossing logic (segment intersection in a local tangent plane).

use super::airports::AirportDb;
use super::flow::gc_dist;
use super::nav::NavData;

const NM_PER_DEG: f64 = 60.0;
const DENSIFY_STEP_NM: f64 = 40.0;
/// A crossing must lie within this bearing (deg) of an airborne aircraft's heading.
const AHEAD_TOL_DEG: f64 = 100.0;

/// Where a route crosses an FCA line.
#[derive(Debug, Clone)]
pub struct FcaCrossing {
    pub lat: f64,
    pub lon: f64,
    /// Distance along the (remaining, for airborne) route to the crossing, nm.
    pub along_nm: f64,
}

/// Resolve the filed route to lat/lon anchors: departure → expanded enroute
/// (fixes/navaids/airways/SID/STAR) → arrival, via the nav engine ([`NavData::build_anchors`]).
fn route_anchors(
    nav: &NavData,
    airports: &AirportDb,
    dep: &str,
    arr: &str,
    route: &str,
) -> Vec<[f64; 2]> {
    nav.build_anchors(airports, dep, arr, route)
        .anchors
        .into_iter()
        .map(|a| a.ll)
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

/// Trim anchors already behind an airborne aircraft; prepend its current position.
fn remaining_anchors(anchors: &[[f64; 2]], lat: f64, lon: f64, hdg: f64) -> Vec<[f64; 2]> {
    if anchors.len() < 2 {
        return anchors.to_vec();
    }
    // Nearest leg by perpendicular distance in a local frame centred on the aircraft.
    let mut best_leg = 0usize;
    let mut best_xt = f64::MAX;
    let mut best_along = 0.0;
    for i in 0..anchors.len() - 1 {
        let a = local([lat, lon], anchors[i]);
        let b = local([lat, lon], anchors[i + 1]);
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
        anchors[best_leg][0],
        anchors[best_leg][1],
        anchors[best_leg + 1][0],
        anchors[best_leg + 1][1],
    );
    let mut idx = if best_along >= leg_len - 3.0 {
        best_leg + 1
    } else {
        best_leg
    };
    // Skip fixes clearly behind the current heading.
    while idx < anchors.len() - 1 {
        let d = gc_dist(lat, lon, anchors[idx][0], anchors[idx][1]);
        let brg = bearing_deg([lat, lon], anchors[idx]);
        if d > 5.0 && angle_diff(hdg, brg) > 110.0 {
            idx += 1;
        } else {
            break;
        }
    }
    let mut out = vec![[lat, lon]];
    out.extend_from_slice(&anchors[idx..]);
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
        anchors
    };
    (path.len() >= 2).then_some(path)
}

/// Where a pre-resolved `path` crosses the FCA line (airborne crossings must be ahead).
pub fn crosses(
    path: &[[f64; 2]],
    fca_points: &[[f64; 2]],
    airborne: bool,
    lat: f64,
    lon: f64,
    hdg: i64,
) -> Option<FcaCrossing> {
    let cross = path_crossing(path, fca_points)?;
    if airborne {
        let brg = bearing_deg([lat, lon], [cross.lat, cross.lon]);
        if angle_diff(hdg as f64, brg) > AHEAD_TOL_DEG {
            return None;
        }
    }
    Some(cross)
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
    crosses(&path, fca_points, gs >= 50, lat, lon, hdg)
}

// --- metering (sequence crossing traffic) ---

/// One aircraft to sequence across the FCA.
pub struct MeterInput {
    /// Unmetered ETA to the crossing, epoch millis.
    pub eta_ms: i64,
    /// In AUTO mode airborne aircraft are fixed constraints (never delayed) and ground floats into
    /// gaps; in MANUAL mode the controller's order rules and airborne crossings are chained too.
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
        // Auto: pinned (airborne + frozen) first, then advisory ground floats.
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
            sched[i] = if c.airborne {
                c.eta_ms
            } else if let Some(f) = c.frozen_ms {
                f
            } else {
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

fn slerp(a: [f64; 2], b: [f64; 2], f: f64) -> [f64; 2] {
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

fn bearing_deg(a: [f64; 2], b: [f64; 2]) -> f64 {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn airports() -> AirportDb {
        // KJFK (New York) and KIAD (Washington Dulles), roughly.
        HashMap::from([
            ("KJFK".to_string(), (40.64, -73.78)),
            ("KIAD".to_string(), (38.95, -77.46)),
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
            ("KJFK".to_string(), (40.64, -73.78)),
            ("KDCA".to_string(), (38.85, -77.04)),
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
        // Aircraft already west of the line, heading further west (270°) → crossing behind.
        let c = crossing_for(&fca, &nav, &ap, "KJFK", "KIAD", "", 39.2, -76.5, 270, 400);
        assert!(c.is_none(), "an aircraft past the line shouldn't match");
    }
}
