//! Runway Balancer engine — assign live arrivals to landing runways and bin the demand.
//! Ported from vatflow's `runway-balancer.html`, but computed server-side against OIS's
//! accurate arrival ETAs (climb-profile + winds) so every controller shares one picture.
//!
//! Priority per arrival: aircraft override → STAR→runway rule → AUTO (least-congested
//! runway near that ETA). Demand is binned into 10-minute buckets per runway.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::airports::AirportDb;
use super::flow::{arrival_gate, gc_dist};
use super::trajectory;
use super::vatsim::VatsimData;
use super::winds::Winds;

/// Demand bins are 10 minutes wide; a bin at/above these counts is near/over capacity.
const BIN_MIN: i64 = 10;
const DEMAND_YELLOW: i32 = 4;
const DEMAND_RED: i32 = 6;
/// A runway with at most this many in a bin can absorb a rebalanced arrival.
const DEMAND_OPEN: i32 = 2;
/// Preset activates ends whose heading is within this of the approach direction.
const PRESET_TOL: f64 = 65.0;

/// A manually-added runway end (for fields the bundled dataset lacks).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CustomEnd {
    pub id: String,
    pub hdg: i32,
    #[serde(default)]
    pub len: i32,
}

/// One landing/departure runway end.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RunwayEnd {
    /// e.g. `04L`.
    pub id: String,
    /// True heading, degrees.
    pub hdg: i32,
    /// Length, ft.
    pub len: i32,
    /// Selected as an active landing runway.
    pub active: bool,
    /// The runway pair this end belongs to, e.g. `04L/22R`.
    pub pair: String,
}

/// One inbound arrival with its assigned runway.
#[derive(Debug, Serialize, ToSchema)]
pub struct RunwayArrival {
    pub cs: String,
    pub dep: String,
    pub actype: String,
    /// Arrival STAR/gate (base name), if detected.
    pub star: Option<String>,
    pub eta: DateTime<Utc>,
    pub dist_nm: i64,
    /// Assigned runway id (null if no active runways).
    pub rwy: Option<String>,
    /// How the runway was chosen: `man` | `star` | `auto`.
    pub src: String,
}

/// A suggestion to move an arrival off a congested runway.
#[derive(Debug, Serialize, ToSchema)]
pub struct RunwayRec {
    pub cs: String,
    /// Runway the aircraft should move to.
    pub to_rwy: String,
    /// Congestion level being relieved: `yellow` | `red`.
    pub level: String,
}

/// Per-runway demand: count and level per 10-min bin.
#[derive(Debug, Serialize, ToSchema)]
pub struct RunwayDemand {
    pub id: String,
    pub bins: Vec<i32>,
    /// `green` | `yellow` | `red` per bin.
    pub levels: Vec<String>,
}

/// The full runway-balancer board for one airport.
#[derive(Debug, Serialize, ToSchema)]
pub struct RunwayBoard {
    pub icao: String,
    /// `built-in` (dataset) or `none — add ends manually`.
    pub source: String,
    pub ends: Vec<RunwayEnd>,
    /// Manually-added ends stored for this airport (also merged into `ends`).
    pub custom_ends: Vec<CustomEnd>,
    #[schema(value_type = std::collections::HashMap<String, String>)]
    pub star_rules: HashMap<String, String>,
    #[schema(value_type = std::collections::HashMap<String, String>)]
    pub overrides: HashMap<String, String>,
    pub window_min: i64,
    pub arrivals: Vec<RunwayArrival>,
    pub demand: Vec<RunwayDemand>,
    /// Rebalance suggestions — aircraft that should move off a congested runway.
    pub recs: Vec<RunwayRec>,
    /// Number of 10-min bins in the demand window.
    pub bins: usize,
    /// Latest raw METAR for the field (server-fetched, cached ~10 min).
    pub metar: Option<String>,
    /// Flight category from the METAR: `VFR` | `MVFR` | `IFR` | `LIFR`.
    pub flight_category: Option<String>,
    /// Human wind, e.g. `270@15G25kt`.
    pub wind: Option<String>,
}

/// Saved runway configuration for an airport. Every field is optional: a `PUT` updates only
/// the fields it carries and leaves the rest of the shared config untouched (server-side
/// `coalesce`), so one controller toggling a runway can't clobber another's STAR rule.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RunwayConfigRequest {
    /// When present, replaces the active runway ends; omit to keep them unchanged.
    pub active_ends: Option<Vec<String>>,
    /// When present, replaces the STAR→runway rules; omit to keep them unchanged.
    #[schema(value_type = Option<std::collections::HashMap<String, String>>)]
    pub star_rules: Option<HashMap<String, String>>,
    /// When present, replaces the per-aircraft overrides; omit to keep them unchanged.
    #[schema(value_type = Option<std::collections::HashMap<String, String>>)]
    pub overrides: Option<HashMap<String, String>>,
    /// When present, replaces the demand horizon; omit to keep it unchanged.
    pub window_min: Option<i32>,
    /// When present, replaces the stored manual ends; omit to keep them unchanged.
    pub custom_ends: Option<Vec<CustomEnd>>,
}

/// A named, reusable runway configuration (e.g. "West Ops").
#[derive(Debug, Serialize, ToSchema)]
pub struct SavedRunwayConfig {
    pub name: String,
    pub active_ends: Vec<String>,
    #[schema(value_type = std::collections::HashMap<String, String>)]
    pub star_rules: HashMap<String, String>,
}

/// Body for saving a named config.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SavedConfigRequest {
    #[serde(default)]
    pub active_ends: Vec<String>,
    #[serde(default)]
    #[schema(value_type = std::collections::HashMap<String, String>)]
    pub star_rules: HashMap<String, String>,
}

/// A live inbound arrival (before runway assignment).
pub struct Arrival {
    pub cs: String,
    pub dep: String,
    pub actype: String,
    pub star: Option<String>,
    pub eta_ms: i64,
    pub dist_nm: f64,
}

/// STAR base name — strip the trailing revision (`CAMRN4` → `CAMRN`, `DOTSS2A` → `DOTSS`).
pub fn star_base(name: &str) -> String {
    let up = name.trim().to_ascii_uppercase();
    let b = up.as_bytes();
    let n = b.len();
    if n >= 2 && b[n - 1].is_ascii_alphabetic() && b[n - 2].is_ascii_digit() {
        return up[..n - 2].to_string();
    }
    if n >= 1 && b[n - 1].is_ascii_digit() {
        return up[..n - 1].to_string();
    }
    up
}

fn angle_diff(a: f64, b: f64) -> f64 {
    (((a - b) + 540.0) % 360.0 - 180.0).abs()
}

/// Activate the ends facing a preset approach direction (`W`/`E`/`N`/`S`), or clear all
/// (`OFF`/anything else). WEST = landing westbound (heading ~270 ± 65°).
pub fn apply_preset(ends: &mut [RunwayEnd], preset: &str) {
    let target = match preset {
        "W" => 270.0,
        "E" => 90.0,
        "N" => 360.0,
        "S" => 180.0,
        _ => {
            ends.iter_mut().for_each(|e| e.active = false);
            return;
        }
    };
    for e in ends.iter_mut() {
        e.active = angle_diff(e.hdg as f64, target) <= PRESET_TOL;
    }
}

/// Collect airborne arrivals to `icao` within the window, with accurate climb-profile +
/// winds ETAs (the same model `flow::compute` uses) and their detected STAR.
pub fn collect_arrivals(
    icao: &str,
    data: &VatsimData,
    airports: &AirportDb,
    winds: &Winds,
    now: DateTime<Utc>,
    window_min: i64,
) -> Vec<Arrival> {
    let icao = icao.to_ascii_uppercase();
    let Some(&(alat, alon)) = airports.get(&icao) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for p in &data.pilots {
        let Some(fp) = &p.flight_plan else { continue };
        if fp.arrival.to_ascii_uppercase() != icao {
            continue;
        }
        if p.groundspeed < 50 {
            continue; // airborne only
        }
        let dist = gc_dist(p.latitude, p.longitude, alat, alon);
        if dist < 3.0 {
            continue; // on the field / rolling out
        }
        let cruise = trajectory::parse_alt_ft(&fp.altitude);
        let tas = trajectory::tas_or_default(fp.cruise_tas.parse().unwrap_or(0.0), cruise);
        let hw = winds.route_headwind(&[[p.latitude, p.longitude], [alat, alon]], cruise);
        let ete_sec = trajectory::profile_transit_sec(dist, p.altitude as f64, cruise, tas, hw);
        let eta = now + Duration::seconds(ete_sec as i64);
        if (eta - now).num_minutes() > window_min {
            continue;
        }
        out.push(Arrival {
            cs: p.callsign.clone(),
            dep: fp.departure.clone(),
            actype: fp.aircraft_short.clone(),
            star: arrival_gate(&fp.route, &icao).map(|s| star_base(&s)),
            eta_ms: eta.timestamp_millis(),
            dist_nm: dist,
        });
    }
    out.sort_by_key(|a| a.eta_ms);
    out
}

/// Assign each arrival (ETA order) a runway: override → STAR rule → AUTO. AUTO picks the
/// active end with the fewest neighbours near this ETA (5 then 15 min), then fewest total.
/// Returns `(runway_id, source)` aligned with `arrivals`.
pub fn assign(
    arrivals: &[Arrival],
    active_ids: &[String],
    rules: &HashMap<String, String>,
    overrides: &HashMap<String, String>,
) -> Vec<(Option<String>, &'static str)> {
    let mut load: HashMap<&str, Vec<i64>> = active_ids
        .iter()
        .map(|s| (s.as_str(), Vec::new()))
        .collect();
    let mut out = Vec::with_capacity(arrivals.len());

    for a in arrivals {
        let mut rwy: Option<String> = None;
        let mut src = "auto";

        if let Some(m) = overrides.get(&a.cs)
            && load.contains_key(m.as_str())
        {
            rwy = Some(m.clone());
            src = "man";
        }
        if rwy.is_none()
            && let Some(star) = &a.star
            && let Some(r) = rules.get(star)
            && load.contains_key(r.as_str())
        {
            rwy = Some(r.clone());
            src = "star";
        }
        if rwy.is_none() && !active_ids.is_empty() {
            let mut best: Option<&str> = None;
            let mut best_score = i64::MAX;
            for id in active_ids {
                let etas = &load[id.as_str()];
                let near5 = etas
                    .iter()
                    .filter(|t| (**t - a.eta_ms).abs() < 5 * 60_000)
                    .count() as i64;
                let near15 = etas
                    .iter()
                    .filter(|t| (**t - a.eta_ms).abs() < 15 * 60_000)
                    .count() as i64;
                let score = near5 * 100 + near15 * 10 + etas.len() as i64;
                if score < best_score {
                    best_score = score;
                    best = Some(id.as_str());
                }
            }
            rwy = best.map(String::from);
            src = "auto";
        }

        if let Some(r) = &rwy
            && let Some(etas) = load.get_mut(r.as_str())
        {
            etas.push(a.eta_ms);
        }
        out.push((rwy, src));
    }
    out
}

fn bin_level(n: i32) -> &'static str {
    if n >= DEMAND_RED {
        "red"
    } else if n >= DEMAND_YELLOW {
        "yellow"
    } else {
        "green"
    }
}

/// Per-runway 10-min demand bins over the window, from `(runway, eta_ms)` assignments.
pub fn demand_bins(
    assigned: &[(Option<String>, i64)],
    active_ids: &[String],
    now_ms: i64,
    window_min: i64,
) -> (Vec<RunwayDemand>, usize) {
    let bins = (window_min as f64 / BIN_MIN as f64).ceil() as usize;
    let mut counts: HashMap<&str, Vec<i32>> = active_ids
        .iter()
        .map(|s| (s.as_str(), vec![0; bins]))
        .collect();

    for (rwy, eta_ms) in assigned {
        let Some(r) = rwy else { continue };
        let Some(c) = counts.get_mut(r.as_str()) else {
            continue;
        };
        let b = (eta_ms - now_ms) / (BIN_MIN * 60_000);
        if b >= 0 && (b as usize) < bins {
            c[b as usize] += 1;
        }
    }

    let demand = active_ids
        .iter()
        .map(|id| {
            let counts = counts.remove(id.as_str()).unwrap_or_else(|| vec![0; bins]);
            let levels = counts.iter().map(|&n| bin_level(n).to_string()).collect();
            RunwayDemand {
                id: id.clone(),
                bins: counts,
                levels,
            }
        })
        .collect();
    (demand, bins)
}

/// Rebalance suggestions: bin by bin, when a runway's 10-min bin is yellow/red, move its
/// *latest* non-manual arrivals to another active runway with an open slot (≤2) in that same
/// bin. Projected counts mutate as suggestions are placed, so we don't over-recommend.
/// `items` is `(callsign, assigned_runway, source, eta_ms)` per arrival.
pub fn recommendations(
    items: &[(String, Option<String>, String, i64)],
    active_ids: &[String],
    now_ms: i64,
    window_min: i64,
) -> Vec<RunwayRec> {
    let bins = (window_min as f64 / BIN_MIN as f64).ceil() as usize;
    let bin_of = |eta: i64| (eta - now_ms) / (BIN_MIN * 60_000);

    // runway -> bin -> [(callsign, source, eta_ms)]
    type BinGrid = HashMap<String, Vec<Vec<(String, String, i64)>>>;
    let mut counts: HashMap<String, Vec<i32>> = active_ids
        .iter()
        .map(|s| (s.clone(), vec![0; bins]))
        .collect();
    let mut in_bin: BinGrid = active_ids
        .iter()
        .map(|s| (s.clone(), vec![Vec::new(); bins]))
        .collect();

    for (cs, rwy, src, eta) in items {
        let Some(r) = rwy else { continue };
        if !counts.contains_key(r) {
            continue;
        }
        let b = bin_of(*eta);
        if b >= 0 && (b as usize) < bins {
            let b = b as usize;
            counts.get_mut(r).unwrap()[b] += 1;
            in_bin.get_mut(r).unwrap()[b].push((cs.clone(), src.clone(), *eta));
        }
    }

    let mut recs: Vec<RunwayRec> = Vec::new();
    for b in 0..bins {
        for e in active_ids {
            let level = bin_level(counts[e][b]);
            if level == "green" {
                continue;
            }
            // Movable = non-manual arrivals in this bin, latest ETA first (least disruptive).
            let mut movable: Vec<(String, i64)> = in_bin[e][b]
                .iter()
                .filter(|(_, src, _)| src != "man")
                .map(|(cs, _, eta)| (cs.clone(), *eta))
                .collect();
            movable.sort_by_key(|m| std::cmp::Reverse(m.1));

            for (cs, _) in movable {
                if bin_level(counts[e][b]) == "green" {
                    break; // relieved enough
                }
                // A different active runway with an open slot in the same bin.
                let target = active_ids
                    .iter()
                    .find(|t| *t != e && counts[*t][b] <= DEMAND_OPEN)
                    .cloned();
                let Some(target) = target else { break };
                recs.push(RunwayRec {
                    cs,
                    to_rwy: target.clone(),
                    level: level.to_string(),
                });
                counts.get_mut(e).unwrap()[b] -= 1;
                counts.get_mut(&target).unwrap()[b] += 1;
            }
        }
    }
    recs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn end(id: &str, hdg: i32) -> RunwayEnd {
        RunwayEnd {
            id: id.into(),
            hdg,
            len: 10000,
            active: false,
            pair: format!("{id}/x"),
        }
    }

    fn arr(cs: &str, star: Option<&str>, eta_ms: i64) -> Arrival {
        Arrival {
            cs: cs.into(),
            dep: "KX".into(),
            actype: "B738".into(),
            star: star.map(|s| s.into()),
            eta_ms,
            dist_nm: 40.0,
        }
    }

    #[test]
    fn star_base_strips_revision() {
        assert_eq!(star_base("CAMRN4"), "CAMRN");
        assert_eq!(star_base("DOTSS2A"), "DOTSS");
        assert_eq!(star_base("lendy6"), "LENDY");
        assert_eq!(star_base("PARCH"), "PARCH");
    }

    #[test]
    fn preset_activates_facing_ends() {
        // KJFK-ish: 04L (044°) and 22R (224°). WEST (270°) activates 22R, not 04L.
        let mut ends = vec![end("04L", 44), end("22R", 224), end("31L", 314)];
        apply_preset(&mut ends, "W");
        assert!(!ends[0].active, "04L faces east, not active for WEST");
        assert!(ends[1].active, "22R faces west");
        assert!(ends[2].active, "31L (314°) within 65° of 270");
        apply_preset(&mut ends, "OFF");
        assert!(ends.iter().all(|e| !e.active));
    }

    #[test]
    fn assign_precedence_override_then_star_then_auto() {
        let active = vec!["04L".to_string(), "04R".to_string()];
        let rules = HashMap::from([("CAMRN".to_string(), "04R".to_string())]);
        let overrides = HashMap::from([("OVR1".to_string(), "04L".to_string())]);
        let arrivals = vec![
            arr("OVR1", Some("CAMRN"), 0),      // override beats the STAR rule
            arr("STR1", Some("CAMRN"), 60_000), // STAR rule → 04R
            arr("AUT1", None, 120_000),         // AUTO
        ];
        let out = assign(&arrivals, &active, &rules, &overrides);
        assert_eq!(out[0], (Some("04L".to_string()), "man"));
        assert_eq!(out[1], (Some("04R".to_string()), "star"));
        assert_eq!(out[2].1, "auto");
        assert!(out[2].0.is_some());
    }

    #[test]
    fn auto_balances_across_runways() {
        // Four arrivals clustered at the same ETA, two empty runways → 2 each.
        let active = vec!["04L".to_string(), "04R".to_string()];
        let rules = HashMap::new();
        let overrides = HashMap::new();
        let arrivals: Vec<Arrival> = (0..4)
            .map(|i| arr(&format!("A{i}"), None, i * 1000))
            .collect();
        let out = assign(&arrivals, &active, &rules, &overrides);
        let left = out
            .iter()
            .filter(|(r, _)| r.as_deref() == Some("04L"))
            .count();
        let right = out
            .iter()
            .filter(|(r, _)| r.as_deref() == Some("04R"))
            .count();
        assert_eq!((left, right), (2, 2), "AUTO should split evenly");
    }

    #[test]
    fn recommends_moving_latest_off_a_crowded_runway() {
        // 04L holds 5 arrivals (yellow) in bin 0; 04R is empty. Recommend moving the
        // latest one(s) to 04R until 04L drops below yellow.
        let active = vec!["04L".to_string(), "04R".to_string()];
        let mut items: Vec<(String, Option<String>, String, i64)> = (0..5)
            .map(|i| {
                (
                    format!("A{i}"),
                    Some("04L".to_string()),
                    "auto".to_string(),
                    i * 1000,
                )
            })
            .collect();
        // A manual assignment must never be recommended for a move.
        items[0].2 = "man".to_string();
        let recs = recommendations(&items, &active, 0, 90);
        assert!(
            !recs.is_empty(),
            "should suggest moving off the crowded runway"
        );
        assert!(recs.iter().all(|r| r.to_rwy == "04R"));
        assert!(
            !recs.iter().any(|r| r.cs == "A0"),
            "manual A0 must not be moved"
        );
        // The moved aircraft are the latest ETAs (highest index).
        assert!(recs.iter().any(|r| r.cs == "A4"));
    }

    #[test]
    fn demand_bins_count_and_level() {
        let active = vec!["04L".to_string()];
        // 6 arrivals all in the first 10-min bin → red.
        let assigned: Vec<(Option<String>, i64)> = (0..6)
            .map(|i| (Some("04L".to_string()), i * 1000))
            .collect();
        let (demand, bins) = demand_bins(&assigned, &active, 0, 90);
        assert_eq!(bins, 9);
        assert_eq!(demand[0].bins[0], 6);
        assert_eq!(demand[0].levels[0], "red");
        assert_eq!(demand[0].levels[1], "green");
    }
}
