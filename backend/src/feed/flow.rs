//! Arrival-flow computation for one airport — classifies inbound traffic (airborne /
//! ground / proposed), estimates ETAs, and meters demand against a program's AAR.
//! Ported from vatflow's `computeFlow`, minus the winds-aloft and CFR-scheduling layers.

use std::collections::HashMap;
use std::sync::LazyLock;

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use utoipa::ToSchema;

use super::airports::AirportDb;
use super::vatsim::VatsimData;

/// Nominal arrival-stream groundspeed used to convert miles-in-trail to a time gap.
const MIT_NOMINAL_KT: f64 = 360.0; // 6 nm/min

/// A per-gate spacing rule (subset of `GateRule` the scheduler needs).
pub struct GateSpacing {
    pub name: String,
    pub trail: i32,
    pub mit: i32,
}

/// The program fields that affect metering. Built from a `ProgramBody` by the handler.
pub struct ProgramInputs {
    pub aar: i32,
    /// Airport-wide minutes-in-trail default.
    pub trail: i32,
    /// Airport-wide miles-in-trail (overrides `trail` when > 0).
    pub mit: i32,
    pub gates: Vec<GateSpacing>,
    pub exclude_wake: Vec<String>,
    pub exclude_types: Vec<String>,
    pub jets_only: bool,
}

#[derive(Debug, Default, Serialize, ToSchema)]
pub struct FlowFlight {
    pub callsign: String,
    pub dep: String,
    pub aircraft_type: String,
    /// Arrival gate (STAR/fix) derived from the filed route; null if none matched.
    pub gate: Option<String>,
    /// `airborne` | `ground` | `proposed` | `arrived`.
    pub status: String,
    pub distance_nm: Option<f64>,
    pub eta: Option<DateTime<Utc>>,
    pub groundspeed: i64,
    /// True when a program excludes this aircraft from metering (still shown).
    pub excluded: bool,
    /// Metered (scheduled) time of arrival after CFR/EDCT sequencing; null if unmetered.
    pub sta: Option<DateTime<Utc>>,
    /// Metering delay in minutes (0 if none / unmetered).
    pub delay_min: i64,
    /// Sequence number in the metered arrival order; null if unmetered.
    pub seq: Option<i64>,
    /// Proposed wheels-up (EDCT / Call-For-Release) for ground & proposed flights.
    pub cfr: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Flow {
    pub icao: String,
    /// Program AAR, if a program exists for this airport.
    pub aar: Option<i32>,
    pub inbound: usize,
    pub airborne: usize,
    pub ground: usize,
    pub proposed: usize,
    /// Metered arrivals estimated to land within the next 60 minutes.
    pub demand_60min: usize,
    /// `demand_60min > aar` when a program exists, else null.
    pub over_capacity: Option<bool>,
    pub flights: Vec<FlowFlight>,
}

#[derive(Clone, Copy, PartialEq)]
enum Engine {
    Piston,
    Turboprop,
    Jet,
}

/// Compute a full arrival picture for `icao`. `icao` must already be uppercase.
pub fn compute(
    icao: &str,
    program: Option<&ProgramInputs>,
    data: &VatsimData,
    airports: &AirportDb,
    now: DateTime<Utc>,
) -> Flow {
    let arr = airports.get(icao).copied();
    let mut flights: Vec<FlowFlight> = Vec::new();
    // Estimated departure time (ms) per flight, aligned with `flights`; used to back out
    // wheels-up (CFR) from the metered STA. None for airborne/arrived (already flying).
    let mut etd_ms: Vec<Option<i64>> = Vec::new();

    for p in &data.pilots {
        let Some(fp) = &p.flight_plan else { continue };
        if fp.arrival.to_ascii_uppercase() != icao {
            continue;
        }
        let (ty, wake) = fp.aircraft_type_wake();
        let dep = fp.departure.to_ascii_uppercase();
        let gate = arrival_gate(&fp.route, icao);
        let excluded = program.is_some_and(|pg| is_excluded(&ty, &wake, pg));

        let dist_to_arr = arr.map(|(alat, alon)| gc_dist(p.latitude, p.longitude, alat, alon));
        let airborne = p.groundspeed > 60 && p.altitude > 300;
        let arrived = !airborne && p.groundspeed <= 60 && dist_to_arr.is_some_and(|d| d < 5.0);

        if arrived {
            flights.push(FlowFlight {
                callsign: p.callsign.clone(),
                dep,
                aircraft_type: ty,
                gate: gate.clone(),
                status: "arrived".into(),
                distance_nm: dist_to_arr,
                eta: Some(now),
                groundspeed: p.groundspeed,
                excluded,
                ..Default::default()
            });
            etd_ms.push(None);
        } else if airborne && dist_to_arr.is_some() {
            let dist = dist_to_arr.unwrap();
            let gs = (p.groundspeed.max(120)) as f64;
            let pad = if dist > 40.0 {
                4.0
            } else if dist > 15.0 {
                2.0
            } else {
                0.0
            };
            let ete_min = (dist / gs) * 60.0 + pad;
            flights.push(FlowFlight {
                callsign: p.callsign.clone(),
                dep,
                aircraft_type: ty,
                gate: gate.clone(),
                status: "airborne".into(),
                distance_nm: Some(dist),
                eta: Some(now + minutes(ete_min)),
                groundspeed: p.groundspeed,
                excluded,
                ..Default::default()
            });
            etd_ms.push(None);
        } else {
            // On the ground (or position-less): estimate a full route flight time.
            let (route_nm, ft_min) = ground_estimate(&dep, arr, fp, airports);
            flights.push(FlowFlight {
                callsign: p.callsign.clone(),
                dep,
                aircraft_type: ty,
                gate,
                status: "ground".into(),
                distance_nm: Some(route_nm),
                eta: Some(now + minutes(ft_min)),
                groundspeed: p.groundspeed,
                excluded,
                ..Default::default()
            });
            // Assumed ready-to-go now; CFR delay (if any) is the only hold.
            etd_ms.push(Some(now.timestamp_millis()));
        }
    }

    for pf in &data.prefiles {
        let Some(fp) = &pf.flight_plan else { continue };
        if fp.arrival.to_ascii_uppercase() != icao {
            continue;
        }
        if flights.iter().any(|f| f.callsign == pf.callsign) {
            continue;
        }
        let (ty, wake) = fp.aircraft_type_wake();
        let dep = fp.departure.to_ascii_uppercase();
        let gate = arrival_gate(&fp.route, icao);
        let excluded = program.is_some_and(|pg| is_excluded(&ty, &wake, pg));
        let (route_nm, ft_min) = ground_estimate(&dep, arr, fp, airports);
        let etd = proposed_etd(&fp.deptime, now);
        flights.push(FlowFlight {
            callsign: pf.callsign.clone(),
            dep,
            aircraft_type: ty,
            gate,
            status: "proposed".into(),
            distance_nm: Some(route_nm),
            eta: Some(etd + minutes(ft_min)),
            groundspeed: 0,
            excluded,
            ..Default::default()
        });
        etd_ms.push(Some(etd.timestamp_millis()));
    }

    // Metering runs only when a program (AAR) exists — it's the opt-in TMU feature.
    if let Some(pg) = program {
        apply_metering(&mut flights, &etd_ms, pg, now);
    }

    let horizon = now + Duration::hours(1);
    let airborne = flights.iter().filter(|f| f.status == "airborne").count();
    let ground = flights.iter().filter(|f| f.status == "ground").count();
    let proposed = flights.iter().filter(|f| f.status == "proposed").count();
    let inbound = airborne + ground + proposed; // excludes already-arrived
    let demand_60min = flights
        .iter()
        .filter(|f| f.status != "arrived" && !f.excluded)
        .filter(|f| f.eta.is_some_and(|e| e >= now && e <= horizon))
        .count();

    Flow {
        icao: icao.to_string(),
        aar: program.map(|p| p.aar),
        inbound,
        airborne,
        ground,
        proposed,
        demand_60min,
        over_capacity: program.map(|p| demand_60min as i32 > p.aar),
        flights,
    }
}

fn is_excluded(ty: &str, wake: &str, pg: &ProgramInputs) -> bool {
    if !wake.is_empty() && pg.exclude_wake.iter().any(|w| w == wake) {
        return true;
    }
    if !pg.exclude_types.is_empty() && pg.exclude_types.iter().any(|t| t == ty) {
        return true;
    }
    if pg.jets_only {
        match engine(ty) {
            Some(Engine::Piston) => return true,
            None if wake == "L" => return true,
            _ => {}
        }
    }
    false
}

/// Same-route in-trail interval (ms) from a spacing rule: MIT (converted at the nominal
/// arrival speed) if set, else minutes-in-trail — never below the runway interval.
fn route_interval_ms(trail: i32, mit: i32, runway_ms: f64) -> f64 {
    let ri = if mit > 0 {
        (mit as f64 / MIT_NOMINAL_KT) * 3_600_000.0
    } else {
        trail as f64 * 60_000.0
    };
    runway_ms.max(ri)
}

/// In-trail interval (ms) for a specific arrival gate: a matching per-gate rule wins,
/// else the program-wide trail/MIT, never below the runway interval.
fn gate_spacing_ms(pg: &ProgramInputs, runway_ms: f64, gate: &str) -> f64 {
    if let Some(g) = pg.gates.iter().find(|g| g.name == gate) {
        return route_interval_ms(g.trail, g.mit, runway_ms);
    }
    route_interval_ms(pg.trail, pg.mit, runway_ms)
}

fn eta_ms(f: &FlowFlight, now_ms: i64) -> i64 {
    f.eta.map(|e| e.timestamp_millis()).unwrap_or(now_ms)
}

/// Two-tier CFR/EDCT scheduler (ported from vatflow). Airborne traffic flies its ETA and
/// is only delayed by runway + same-gate in-trail spacing; ground/proposed traffic slots
/// into the remaining capacity, absorbing its delay on the ground as a wheels-up (CFR).
/// Fills `sta`, `delay_min`, `seq`, and `cfr` on each metered flight.
fn apply_metering(
    flights: &mut [FlowFlight],
    etd_ms: &[Option<i64>],
    pg: &ProgramInputs,
    now: DateTime<Utc>,
) {
    let now_ms = now.timestamp_millis();
    let runway = 3_600_000.0 / pg.aar.max(1) as f64; // runway interval (ms)

    let metered: Vec<usize> = (0..flights.len())
        .filter(|&i| !flights[i].excluded && flights[i].status != "arrived")
        .collect();

    let mut airborne: Vec<usize> = metered
        .iter()
        .copied()
        .filter(|&i| flights[i].status == "airborne")
        .collect();
    airborne.sort_by_key(|&i| eta_ms(&flights[i], now_ms));

    let mut ground: Vec<usize> = metered
        .iter()
        .copied()
        .filter(|&i| matches!(flights[i].status.as_str(), "ground" | "proposed"))
        .collect();
    ground.sort_by_key(|&i| eta_ms(&flights[i], now_ms));

    let mut assigned: Vec<(f64, String)> = Vec::new(); // (sta_ms, gate)
    let mut prev_sta = f64::NEG_INFINITY;
    let mut last_gate_sta: HashMap<String, f64> = HashMap::new();

    // Tier 1 — airborne fly their ETA, delayed only by runway + same-gate in-trail.
    for &i in &airborne {
        let eta = eta_ms(&flights[i], now_ms) as f64;
        let mut sta = eta.max(prev_sta + runway);
        if let Some(g) = &flights[i].gate {
            if let Some(&lg) = last_gate_sta.get(g) {
                sta = sta.max(lg + gate_spacing_ms(pg, runway, g));
            }
        }
        write_meter(&mut flights[i], sta, eta, etd_ms[i]);
        prev_sta = sta;
        if let Some(g) = flights[i].gate.clone() {
            last_gate_sta.insert(g, sta);
        }
        assigned.push((sta, flights[i].gate.clone().unwrap_or_default()));
    }

    // Tier 2 — ground/proposed slot into the first gap clear of every assigned slot.
    for &i in &ground {
        let eta = eta_ms(&flights[i], now_ms) as f64;
        let gate = flights[i].gate.clone();
        let mut cand = eta;
        let mut moved = true;
        while moved {
            moved = false;
            for (t, sg) in &assigned {
                let req = match &gate {
                    Some(g) if !sg.is_empty() && g == sg => gate_spacing_ms(pg, runway, g),
                    _ => runway,
                };
                if (cand - t).abs() < req {
                    cand = t + req;
                    moved = true;
                }
            }
        }
        write_meter(&mut flights[i], cand, eta, etd_ms[i]);
        assigned.push((cand, gate.unwrap_or_default()));
    }

    // Sequence numbers follow the metered arrival order.
    let mut order = metered;
    order.sort_by(|&a, &b| {
        let sa = flights[a]
            .sta
            .map(|s| s.timestamp_millis())
            .unwrap_or(i64::MAX);
        let sb = flights[b]
            .sta
            .map(|s| s.timestamp_millis())
            .unwrap_or(i64::MAX);
        sa.cmp(&sb)
    });
    for (n, &i) in order.iter().enumerate() {
        flights[i].seq = Some(n as i64 + 1);
    }
}

/// Write the metering outputs onto a flight: STA, delay, and (for ground/proposed) the
/// wheels-up CFR — which is the original ETD pushed back by the metering delay.
fn write_meter(f: &mut FlowFlight, sta_ms: f64, eta_ms: f64, etd_ms: Option<i64>) {
    let sta = sta_ms as i64;
    f.sta = DateTime::from_timestamp_millis(sta);
    let delay = sta - eta_ms as i64;
    f.delay_min = (delay.max(0) as f64 / 60_000.0).round() as i64;
    if let Some(etd) = etd_ms {
        f.cfr = DateTime::from_timestamp_millis(etd + delay.max(0));
    }
}

/// Route length (nm) and full flight time (min) for a ground/proposed flight.
fn ground_estimate(
    dep: &str,
    arr: Option<(f64, f64)>,
    fp: &super::vatsim::FlightPlan,
    airports: &AirportDb,
) -> (f64, f64) {
    let dep_pt = airports.get(dep).copied();
    let route_nm = match (dep_pt, arr) {
        (Some((dlat, dlon)), Some((alat, alon))) => gc_dist(dlat, dlon, alat, alon) * 1.12,
        _ => 300.0,
    };
    let tas = parse_tas(&fp.cruise_tas);
    let ft_min = (route_nm / tas) * 60.0 + 14.0; // + taxi/climb allowance
    (route_nm, ft_min)
}

/// Estimated departure time for a prefile: filed `deptime` (HHMM Z) nudged into a sane
/// window, else 20 minutes out. Never earlier than 20 minutes from now.
fn proposed_etd(deptime: &str, now: DateTime<Utc>) -> DateTime<Utc> {
    let floor = now + Duration::minutes(20);
    if deptime.len() == 4 && deptime.chars().all(|c| c.is_ascii_digit()) {
        let h: u32 = deptime[0..2].parse().unwrap_or(99);
        let m: u32 = deptime[2..4].parse().unwrap_or(99);
        if h < 24 && m < 60 {
            if let Some(naive) = now.date_naive().and_hms_opt(h, m, 0) {
                let mut t = naive.and_utc();
                if t < now - Duration::hours(2) {
                    t += Duration::hours(24);
                } else if t - now > Duration::hours(12) {
                    t -= Duration::hours(24);
                }
                return t.max(floor);
            }
        }
    }
    floor
}

fn minutes(m: f64) -> Duration {
    Duration::milliseconds((m * 60_000.0) as i64)
}

/// Filed cruise TAS (knots); defaults to 420 for missing/implausible values.
fn parse_tas(raw: &str) -> f64 {
    let n: f64 = raw.trim().parse().unwrap_or(0.0);
    if n >= 60.0 && n <= 1200.0 { n } else { 420.0 }
}

/// Arrival gate (STAR/fix) heuristic ported from vatflow's `arrivalGate`: scan the filed
/// route from the end and return the first token that looks like a 5-letter RNAV fix, a
/// 3-letter navaid, or a STAR/SID name (e.g. `OZZZI4`). `arr` must be uppercase.
fn arrival_gate(route: &str, arr: &str) -> Option<String> {
    if route.trim().is_empty() {
        return None;
    }
    let toks: Vec<String> = route
        .to_ascii_uppercase()
        .split_whitespace()
        .map(|t| {
            t.split('/')
                .next()
                .unwrap_or("")
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
        })
        .collect();
    for t in toks.iter().rev() {
        if t.is_empty() || t == "DCT" || t == arr {
            continue;
        }
        if is_fix(t) || is_navaid(t) || is_star(t) {
            return Some(t.clone());
        }
    }
    None
}

/// Exactly five uppercase letters — a named RNAV fix (CAMRN, LENDY).
fn is_fix(t: &str) -> bool {
    t.len() == 5 && t.bytes().all(|b| b.is_ascii_uppercase())
}

/// Exactly three uppercase letters — a VOR/navaid (JFK).
fn is_navaid(t: &str) -> bool {
    t.len() == 3 && t.bytes().all(|b| b.is_ascii_uppercase())
}

/// A STAR/SID name: 3–5 letters, a digit, then an optional trailing letter (OZZZI4, PARCH3A).
fn is_star(t: &str) -> bool {
    let b = t.as_bytes();
    let letters_ok =
        |s: &[u8]| (3..=5).contains(&s.len()) && s.iter().all(|c| c.is_ascii_uppercase());
    match b.last() {
        Some(c) if c.is_ascii_digit() => letters_ok(&b[..b.len() - 1]),
        Some(c) if c.is_ascii_uppercase() => {
            b.len() >= 5 && b[b.len() - 2].is_ascii_digit() && letters_ok(&b[..b.len() - 2])
        }
        _ => false,
    }
}

/// Great-circle distance in nautical miles.
fn gc_dist(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = 3440.065; // Earth radius in nm
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dphi = (lat2 - lat1).to_radians();
    let dlmb = (lon2 - lon1).to_radians();
    let a = (dphi / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dlmb / 2.0).sin().powi(2);
    2.0 * r * a.sqrt().asin()
}

fn engine(ty: &str) -> Option<Engine> {
    ENGINE_REF.get(ty).copied()
}

// Aircraft engine reference for jets-only metering — ported from vatflow's
// FALLBACK_AIRCRAFT_REF. Not exhaustive; unknown types are treated as includable.
static ENGINE_REF: LazyLock<HashMap<&'static str, Engine>> = LazyLock::new(|| {
    use Engine::*;
    let mut m = HashMap::new();
    let jets = [
        "A10", "A124", "A19N", "A20N", "A21N", "A306", "A310", "A318", "A319", "A320", "A321",
        "A332", "A333", "A339", "A343", "A359", "A388", "B712", "B722", "B732", "B733", "B734",
        "B735", "B736", "B737", "B738", "B739", "B742", "B744", "B748", "B752", "B753", "B762",
        "B763", "B764", "B772", "B773", "B77L", "B77W", "B788", "B789", "B78X", "C25A", "C25B",
        "C510", "C550", "C56X", "C680", "C700", "C750", "CL30", "CL35", "CL60", "CRJ2", "CRJ7",
        "CRJ9", "CRJX", "DC10", "DC93", "E135", "E145", "E170", "E175", "E190", "E195", "E290",
        "E295", "E35L", "E55P", "E75L", "E75S", "F16", "F18", "FA7X", "FA8X", "GALX", "GLF4",
        "GLF5", "GLF6", "H25B", "HDJT", "IL76", "LJ35", "LJ45", "LJ60", "MD11", "PC24", "RJ85",
        "SF50",
    ];
    let turboprops = [
        "AT43", "AT45", "AT46", "AT72", "AT75", "AT76", "BE20", "BE99", "C208", "DH8A", "DH8B",
        "DH8C", "DH8D", "E110", "E120", "E50P", "PA31", "PC12", "SB20", "SF34", "TBM9",
    ];
    let pistons = [
        "BE36", "BE58", "C152", "C172", "C182", "DA20", "DA40", "DA42", "PA28", "PA34", "PA44",
        "PA46", "SR22",
    ];
    for t in jets {
        m.insert(t, Jet);
    }
    for t in turboprops {
        m.insert(t, Turboprop);
    }
    for t in pistons {
        m.insert(t, Piston);
    }
    m
});
