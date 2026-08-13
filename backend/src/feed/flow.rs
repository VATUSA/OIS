//! Arrival-flow computation for one airport — classifies inbound traffic (airborne /
//! ground / proposed), estimates ETAs, and meters demand against a program's AAR.
//! Ported from vatflow's `computeFlow`, minus the winds-aloft and CFR-scheduling layers.

use std::collections::{HashMap, HashSet};
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
    /// True when the CFR has been issued (locked) rather than merely proposed.
    pub cfr_issued: bool,
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

/// A pending departure out of a field (before any metering is applied).
pub struct PendingDep {
    pub callsign: String,
    /// Origin airport ICAO.
    pub dep: String,
    pub arrival: String,
    pub aircraft_type: String,
    pub gate: Option<String>,
    pub status: String, // "ground" | "proposed"
}

/// All pending (not-yet-airborne) departures out of any field in `deps`, regardless of
/// destination. `deps` entries must be uppercase.
pub fn pending_departures(deps: &HashSet<String>, data: &VatsimData) -> Vec<PendingDep> {
    let mut out: Vec<PendingDep> = Vec::new();
    for p in &data.pilots {
        let Some(fp) = &p.flight_plan else { continue };
        let dep = fp.departure.to_ascii_uppercase();
        if !deps.contains(&dep) {
            continue;
        }
        // Already airborne means it has departed — not a pending departure.
        if p.groundspeed > 60 && p.altitude > 300 {
            continue;
        }
        let arrival = fp.arrival.to_ascii_uppercase();
        let (ty, _wake) = fp.aircraft_type_wake();
        out.push(PendingDep {
            callsign: p.callsign.clone(),
            dep,
            gate: arrival_gate(&fp.route, &arrival),
            arrival,
            aircraft_type: ty,
            status: "ground".into(),
        });
    }
    for pf in &data.prefiles {
        let Some(fp) = &pf.flight_plan else { continue };
        let dep = fp.departure.to_ascii_uppercase();
        if !deps.contains(&dep) {
            continue;
        }
        if out.iter().any(|d| d.callsign == pf.callsign) {
            continue;
        }
        let arrival = fp.arrival.to_ascii_uppercase();
        let (ty, _wake) = fp.aircraft_type_wake();
        out.push(PendingDep {
            callsign: pf.callsign.clone(),
            dep,
            gate: arrival_gate(&fp.route, &arrival),
            arrival,
            aircraft_type: ty,
            status: "proposed".into(),
        });
    }
    out
}

/// Compute a full arrival picture for `icao`. `icao` must already be uppercase. `issued`
/// maps callsign -> locked wheels-up for any CFRs already issued into this field.
pub fn compute(
    icao: &str,
    program: Option<&ProgramInputs>,
    data: &VatsimData,
    airports: &AirportDb,
    issued: &HashMap<String, DateTime<Utc>>,
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
        apply_metering(&mut flights, &etd_ms, pg, issued, now);
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
    issued: &HashMap<String, DateTime<Utc>>,
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

    // Ground/proposed, split into flights with a locked (issued) CFR vs auto-slotted.
    let ground_all: Vec<usize> = metered
        .iter()
        .copied()
        .filter(|&i| matches!(flights[i].status.as_str(), "ground" | "proposed"))
        .collect();
    let ground_issued: Vec<usize> = ground_all
        .iter()
        .copied()
        .filter(|&i| issued.contains_key(&flights[i].callsign))
        .collect();
    let mut ground: Vec<usize> = ground_all
        .iter()
        .copied()
        .filter(|&i| !issued.contains_key(&flights[i].callsign))
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

    // Tier 2a — issued CFRs keep their locked wheels-up; the slot is reserved for them.
    for &i in &ground_issued {
        let wheels = issued[&flights[i].callsign];
        let wheels_ms = wheels.timestamp_millis();
        let eta = eta_ms(&flights[i], now_ms);
        let flight_ms = eta - etd_ms[i].unwrap_or(now_ms); // enroute time
        let sta = wheels_ms + flight_ms;
        flights[i].sta = DateTime::from_timestamp_millis(sta);
        flights[i].delay_min = ((sta - eta).max(0) as f64 / 60_000.0).round() as i64;
        flights[i].cfr = Some(wheels);
        flights[i].cfr_issued = true;
        assigned.push((sta as f64, flights[i].gate.clone().unwrap_or_default()));
    }

    // Tier 2b — everything else on the ground slots into the first gap clear of every slot.
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

/// Given a pilot's ready wheels-up, return the earliest runway slot at or after it that is
/// clear of every other metered arrival — i.e. the wheels-up to lock. `icao`/`callsign`
/// uppercase. Returns None if the flight isn't a metered ground/proposed departure.
pub fn ready_time_slot(
    icao: &str,
    program: &ProgramInputs,
    data: &VatsimData,
    airports: &AirportDb,
    issued: &HashMap<String, DateTime<Utc>>,
    callsign: &str,
    ready: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let flow = compute(icao, Some(program), data, airports, issued, now);
    let target = flow.flights.iter().find(|f| f.callsign == callsign)?;
    // enroute time = STA - proposed wheels-up (both already computed for this flight)
    let sta = target.sta?.timestamp_millis();
    let cfr = target.cfr?.timestamp_millis();
    let flight_ms = sta - cfr;
    let runway = 3_600_000.0 / program.aar.max(1) as f64;
    let gate = target.gate.clone();

    // Every other metered arrival's assigned slot.
    let assigned: Vec<(f64, String)> = flow
        .flights
        .iter()
        .filter(|f| {
            f.callsign != callsign && !f.excluded && f.status != "arrived" && f.sta.is_some()
        })
        .map(|f| {
            (
                f.sta.unwrap().timestamp_millis() as f64,
                f.gate.clone().unwrap_or_default(),
            )
        })
        .collect();

    // Slot the flight's arrival from (ready + enroute) forward past any conflicts. Never
    // earlier than now — a release can't be issued in the past.
    let base_ms = ready.timestamp_millis().max(now.timestamp_millis());
    let mut cand = (base_ms + flight_ms) as f64;
    let mut moved = true;
    while moved {
        moved = false;
        for (t, sg) in &assigned {
            let req = match &gate {
                Some(g) if !sg.is_empty() && g == sg => gate_spacing_ms(program, runway, g),
                _ => runway,
            };
            if (cand - t).abs() < req {
                cand = t + req;
                moved = true;
            }
        }
    }
    DateTime::from_timestamp_millis(cand as i64 - flight_ms)
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
pub fn gc_dist(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::vatsim::{FlightPlan, Pilot, Prefile, VatsimData};

    // A fixed reference time so ETA/STA arithmetic is deterministic.
    fn t0() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).unwrap()
    }

    fn base_program() -> ProgramInputs {
        ProgramInputs {
            aar: 30,
            trail: 0,
            mit: 0,
            gates: vec![],
            exclude_wake: vec![],
            exclude_types: vec![],
            jets_only: false,
        }
    }

    fn airports() -> AirportDb {
        // KJFK and KBOS (lat, lon).
        HashMap::from([
            ("KJFK".to_string(), (40.6413, -73.7781)),
            ("KBOS".to_string(), (42.3656, -71.0096)),
        ])
    }

    fn fp(dep: &str, arr: &str, route: &str) -> FlightPlan {
        FlightPlan {
            departure: dep.into(),
            arrival: arr.into(),
            route: route.into(),
            aircraft_short: "B738".into(),
            cruise_tas: "420".into(),
            ..Default::default()
        }
    }

    fn pilot(cs: &str, lat: f64, lon: f64, alt: i64, gs: i64, plan: FlightPlan) -> Pilot {
        Pilot {
            callsign: cs.into(),
            latitude: lat,
            longitude: lon,
            altitude: alt,
            groundspeed: gs,
            heading: 0,
            flight_plan: Some(plan),
        }
    }

    /// A FlowFlight with a given ETA offset (minutes) from t0.
    fn ff(cs: &str, status: &str, eta_min: i64, gate: Option<&str>) -> FlowFlight {
        FlowFlight {
            callsign: cs.into(),
            status: status.into(),
            eta: Some(t0() + Duration::minutes(eta_min)),
            gate: gate.map(Into::into),
            ..Default::default()
        }
    }

    fn min_after(base: DateTime<Utc>, t: Option<DateTime<Utc>>) -> i64 {
        (t.unwrap() - base).num_minutes()
    }

    fn ms(t: Option<DateTime<Utc>>) -> i64 {
        t.unwrap().timestamp_millis()
    }

    // ---- gate derivation ----

    #[test]
    fn gate_predicates() {
        assert!(is_fix("CAMRN"));
        assert!(!is_fix("JFK"));
        assert!(!is_fix("CAMRN4"));

        assert!(is_navaid("JFK"));
        assert!(!is_navaid("CAMRN"));

        assert!(is_star("OZZZI4"));
        assert!(is_star("PARCH3"));
        assert!(is_star("CAMRN4A"));
        assert!(!is_star("CAMRN")); // no digit
        assert!(!is_star("N0450F350")); // interior digits, not a STAR name
        assert!(!is_star("AB3")); // too few letters
    }

    #[test]
    fn arrival_gate_scans_from_the_end() {
        assert_eq!(
            arrival_gate("DCT CAMRN KJFK", "KJFK").as_deref(),
            Some("CAMRN")
        );
        assert_eq!(
            arrival_gate("KBOS PARCH4", "KJFK").as_deref(),
            Some("PARCH4")
        );
        assert_eq!(arrival_gate("DCT JFK", "KJFK").as_deref(), Some("JFK"));
        assert_eq!(arrival_gate("N0450F350 DCT", "KJFK"), None);
        assert_eq!(arrival_gate("", "KJFK"), None);
        assert_eq!(arrival_gate("DCT KJFK", "KJFK"), None); // only the destination
    }

    // ---- geometry / parsing ----

    #[test]
    fn gc_dist_matches_known_distances() {
        // 1 degree of latitude ~= 60 nm.
        assert!((gc_dist(0.0, 0.0, 1.0, 0.0) - 60.0).abs() < 0.5);
        // KJFK -> KBOS is ~162 nm (187 statute miles).
        let d = gc_dist(40.6413, -73.7781, 42.3656, -71.0096);
        assert!((d - 162.0).abs() < 3.0, "got {d}");
        assert_eq!(gc_dist(40.0, -73.0, 40.0, -73.0), 0.0);
    }

    #[test]
    fn parse_tas_defaults_when_implausible() {
        assert_eq!(parse_tas("480"), 480.0);
        assert_eq!(parse_tas(""), 420.0);
        assert_eq!(parse_tas("abc"), 420.0);
        assert_eq!(parse_tas("50"), 420.0); // below floor
        assert_eq!(parse_tas("2000"), 420.0); // above ceiling
    }

    // ---- spacing math ----

    #[test]
    fn route_interval_mit_beats_minutes_and_floors_at_runway() {
        let runway = 60_000.0; // 1 min
        // 20 MIT at 360kt = 200s.
        assert_eq!(route_interval_ms(0, 20, runway), 200_000.0);
        // 5 minutes-in-trail = 300s.
        assert_eq!(route_interval_ms(5, 0, runway), 300_000.0);
        // MIT overrides minutes when both set.
        assert_eq!(route_interval_ms(5, 20, runway), 200_000.0);
        // Never below the runway interval.
        assert_eq!(route_interval_ms(0, 1, 600_000.0), 600_000.0);
    }

    #[test]
    fn gate_spacing_prefers_matching_gate_rule() {
        let runway = 60_000.0;
        let pg = ProgramInputs {
            mit: 15, // airport-wide 15 MIT = 150s, above the runway floor
            gates: vec![GateSpacing {
                name: "CAMRN".into(),
                trail: 0,
                mit: 20,
            }],
            ..base_program()
        };
        // Matching gate rule (20 MIT) wins.
        assert_eq!(gate_spacing_ms(&pg, runway, "CAMRN"), 200_000.0);
        // No matching rule -> airport-wide mit (15 nm).
        let expected = (15.0 / MIT_NOMINAL_KT) * 3_600_000.0;
        assert_eq!(gate_spacing_ms(&pg, runway, "LENDY"), expected);
    }

    #[test]
    fn proposed_etd_uses_deptime_or_floor() {
        use chrono::Timelike;
        // A filed deptime well ahead of now is honored.
        let etd = proposed_etd("2330", t0());
        assert_eq!((etd.hour(), etd.minute()), (23, 30));
        // Blank deptime -> now + 20 minutes.
        let etd = proposed_etd("", t0());
        assert_eq!(etd, t0() + Duration::minutes(20));
    }

    // ---- exclusions ----

    #[test]
    fn exclusions_by_wake_type_and_jets_only() {
        let by_wake = ProgramInputs {
            exclude_wake: vec!["L".into()],
            ..base_program()
        };
        assert!(is_excluded("B738", "L", &by_wake));
        assert!(!is_excluded("B738", "H", &by_wake));

        let by_type = ProgramInputs {
            exclude_types: vec!["C172".into()],
            ..base_program()
        };
        assert!(is_excluded("C172", "", &by_type));
        assert!(!is_excluded("B738", "", &by_type));

        let jets = ProgramInputs {
            jets_only: true,
            ..base_program()
        };
        assert!(is_excluded("C172", "", &jets)); // piston
        assert!(!is_excluded("B738", "", &jets)); // jet
        assert!(is_excluded("ZZZZ", "L", &jets)); // unknown + light wake
        assert!(!is_excluded("ZZZZ", "", &jets)); // unknown, included by default
    }

    // ---- classification (compute) ----

    #[test]
    fn compute_classifies_and_counts() {
        let data = VatsimData {
            pilots: vec![
                // On the ground at KBOS, bound for KJFK.
                pilot(
                    "GRD1",
                    42.36,
                    -71.0,
                    0,
                    0,
                    fp("KBOS", "KJFK", "DCT CAMRN KJFK"),
                ),
                // Airborne ~38 nm south of KJFK.
                pilot(
                    "AIR1",
                    40.0,
                    -73.7781,
                    15_000,
                    300,
                    fp("KBOS", "KJFK", "PARCH4"),
                ),
                // Sitting on the field at KJFK (arrived).
                pilot("ARR1", 40.6413, -73.7781, 0, 0, fp("KBOS", "KJFK", "DCT")),
                // Not our airport.
                pilot("OTH1", 41.0, -73.0, 0, 0, fp("KBOS", "KLGA", "DCT")),
            ],
            prefiles: vec![Prefile {
                callsign: "PRE1".into(),
                flight_plan: Some(fp("KBOS", "KJFK", "DCT LENDY KJFK")),
            }],
            ..Default::default()
        };
        let pg = base_program();
        let flow = compute("KJFK", Some(&pg), &data, &airports(), &HashMap::new(), t0());

        assert_eq!(flow.airborne, 1);
        assert_eq!(flow.ground, 1);
        assert_eq!(flow.proposed, 1);
        assert_eq!(flow.inbound, 3); // excludes the arrived flight

        let by = |cs: &str| flow.flights.iter().find(|f| f.callsign == cs).unwrap();
        assert_eq!(by("GRD1").status, "ground");
        assert_eq!(by("AIR1").status, "airborne");
        assert_eq!(by("AIR1").gate.as_deref(), Some("PARCH4"));
        assert_eq!(by("ARR1").status, "arrived");
        assert_eq!(by("PRE1").status, "proposed");
        // OTH1 is bound for KLGA and should not appear.
        assert!(flow.flights.iter().all(|f| f.callsign != "OTH1"));
    }

    #[test]
    fn pending_departures_lists_ground_and_prefiles() {
        let data = VatsimData {
            pilots: vec![
                pilot("G1", 42.36, -71.0, 0, 0, fp("KBOS", "KJFK", "DCT")),
                // Airborne — already departed, excluded.
                pilot("AIR", 42.5, -71.5, 12_000, 350, fp("KBOS", "KJFK", "DCT")),
                // Departs elsewhere.
                pilot("X", 42.36, -71.0, 0, 0, fp("KLGA", "KJFK", "DCT")),
            ],
            prefiles: vec![Prefile {
                callsign: "P1".into(),
                flight_plan: Some(fp("KBOS", "KMIA", "DCT")),
            }],
            ..Default::default()
        };
        let fields = HashSet::from(["KBOS".to_string()]);
        let deps = pending_departures(&fields, &data);
        let names: Vec<_> = deps.iter().map(|d| d.callsign.as_str()).collect();
        assert!(names.contains(&"G1"));
        assert!(names.contains(&"P1"));
        assert!(!names.contains(&"AIR")); // airborne
        assert!(!names.contains(&"X")); // wrong field
    }

    // ---- scheduler (apply_metering) ----

    #[test]
    fn airborne_spaced_at_runway_interval() {
        let now = t0();
        let mut flights = vec![
            ff("A1", "airborne", 1, None),
            ff("A2", "airborne", 2, None),
            ff("A3", "airborne", 3, None),
        ];
        let etd = vec![None, None, None];
        apply_metering(
            &mut flights,
            &etd,
            &base_program_aar(10),
            &HashMap::new(),
            now,
        );

        // 6-minute runway interval (AAR 10).
        assert_eq!(min_after(now, flights[0].sta), 1);
        assert_eq!(min_after(now, flights[1].sta), 7);
        assert_eq!(min_after(now, flights[2].sta), 13);
        assert_eq!(flights[1].delay_min, 5);
        assert_eq!(flights[2].delay_min, 10);
        assert_eq!(flights[0].seq, Some(1));
        assert_eq!(flights[2].seq, Some(3));
    }

    #[test]
    fn ground_flight_gets_wheels_up_cfr() {
        let now = t0();
        let mut flights = vec![
            ff("AIR", "airborne", 5, None),
            ff("GRD", "ground", 10, None),
        ];
        // Ground flight is assumed ready now.
        let etd = vec![None, Some(now.timestamp_millis())];
        apply_metering(
            &mut flights,
            &etd,
            &base_program_aar(10),
            &HashMap::new(),
            now,
        );

        // AIR at +5, GRD conflicts (within 6 min) so slots to +11.
        assert_eq!(min_after(now, flights[0].sta), 5);
        assert_eq!(min_after(now, flights[1].sta), 11);
        assert_eq!(flights[1].delay_min, 1);
        // wheels-up = etd(now) + delay(1 min).
        assert_eq!(min_after(now, flights[1].cfr), 1);
    }

    #[test]
    fn same_gate_traffic_gets_in_trail_spacing() {
        let now = t0();
        let pg = ProgramInputs {
            aar: 60, // 1-min runway
            gates: vec![GateSpacing {
                name: "CAMRN".into(),
                trail: 0,
                mit: 20, // 200s in-trail
            }],
            ..base_program()
        };
        let mut flights = vec![
            ff("A1", "airborne", 1, Some("CAMRN")),
            ff("A2", "airborne", 2, Some("CAMRN")),
        ];
        let etd = vec![None, None];
        apply_metering(&mut flights, &etd, &pg, &HashMap::new(), now);
        // The gate in-trail (200s) exceeds the runway interval.
        assert_eq!(ms(flights[1].sta) - ms(flights[0].sta), 200_000);
    }

    #[test]
    fn issued_cfr_is_reserved_at_its_locked_wheels_up() {
        let now = t0();
        let wheels = now + Duration::minutes(30);
        let mut flights = vec![ff("GRD", "ground", 10, None)];
        let etd = vec![Some(now.timestamp_millis())];
        let issued = HashMap::from([("GRD".to_string(), wheels)]);
        apply_metering(&mut flights, &etd, &base_program_aar(10), &issued, now);

        assert!(flights[0].cfr_issued);
        assert_eq!(flights[0].cfr, Some(wheels));
    }

    #[test]
    fn ready_time_slot_never_before_ready_or_now() {
        let data = VatsimData {
            pilots: vec![pilot("GRD1", 42.36, -71.0, 0, 0, fp("KBOS", "KJFK", "DCT"))],
            ..Default::default()
        };
        let pg = base_program();
        let ready = t0() + Duration::minutes(60);
        let slot = ready_time_slot(
            "KJFK",
            &pg,
            &data,
            &airports(),
            &HashMap::new(),
            "GRD1",
            ready,
            t0(),
        )
        .unwrap();
        assert!(slot >= ready, "slot {slot} should be >= ready {ready}");
    }

    fn base_program_aar(aar: i32) -> ProgramInputs {
        ProgramInputs {
            aar,
            ..base_program()
        }
    }
}
