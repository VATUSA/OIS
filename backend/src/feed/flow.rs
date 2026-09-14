//! Arrival-flow computation for one airport — classifies inbound traffic (airborne /
//! ground / proposed), estimates ETAs (shared climb-profile + winds model), and meters
//! demand against a program's AAR. Ported from vatflow's `computeFlow`.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use utoipa::ToSchema;

use super::airports::AirportDb;
use super::delays::nearest_runway;
use super::nav::NavData;
use super::predict;
use super::runway_db::RunwayDb;
use super::taxi_estimate;
use super::trajectory;
use super::vatsim::VatsimData;
use super::winds::Winds;
use crate::models::AirportGateBody;

/// A gate/parking spot must be within this of a spawn point to count as a match (#164 sub-issue
/// C/E — matches `feed::taxi_observations`'s own persistence-time gate matching exactly).
const GATE_MATCH_MAX_NM: f64 = 0.06; // ~360 ft

/// Groundspeed above which a pilot's heading reflects actual taxi movement rather than arbitrary
/// gate-parking orientation (#164 sub-issue E). A stationary aircraft's heading is essentially
/// random with respect to any runway, so [`resolve_ground_allowance_sec`] only trusts it — and
/// attempts a runway match — once the aircraft has crossed this threshold; below it, the ladder
/// falls to the airport/default tier instead of risking a coincidental wrong-runway match.
/// Mirrors `taxi_observations`'s own roll-detection threshold (`GS_START`) — the same physical
/// moment, one shared constant.
pub(crate) const TAXI_ROLL_GS_KT: i64 = 7;

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
    /// Estimated wheels-up for ground/proposed inbounds (ready-now or filed ETD); null once
    /// airborne. Used to back out enroute time for GDP EDCTs.
    pub etd: Option<DateTime<Utc>>,
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
#[allow(clippy::too_many_arguments)]
pub fn compute(
    icao: &str,
    program: Option<&ProgramInputs>,
    data: &VatsimData,
    airports: &AirportDb,
    nav: &NavData,
    winds: &Winds,
    profiles: &trajectory::ProfileTable,
    issued: &HashMap<String, DateTime<Utc>>,
    gates: &HashMap<String, Vec<AirportGateBody>>,
    runways: &RunwayDb,
    taxi_samples: &HashMap<String, Vec<taxi_estimate::TaxiSample>>,
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
        let profile = profiles.resolve(&ty, &wake);
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
        } else if airborne && arr.is_some() {
            let (alat, alon) = arr.unwrap();
            let cruise = trajectory::parse_alt_ft(&fp.altitude);
            let cruise_tas =
                trajectory::capped_cruise_tas(parse_tas(&fp.cruise_tas), cruise, profile);
            // Airborne arrival: resolve the filed route and time the remaining along-route
            // distance to the field through the shared predictor (same one FCA metering uses),
            // so the ladder slot matches the aircraft's drawn route on the map.
            let pred = predict::arrival_eta(
                nav,
                airports,
                winds,
                profile,
                &predict::ArrivalInput {
                    dep: &dep,
                    arr: &fp.arrival,
                    route: &fp.route,
                    pos: [p.latitude, p.longitude],
                    alt_ft: p.altitude as f64,
                    gs: p.groundspeed,
                    hdg: p.heading,
                    arr_ll: [alat, alon],
                    cruise_ft: cruise,
                    cruise_tas,
                },
                // Airborne — `eta_along_route` only applies the ground allowance on its ground
                // branch, so this value is structurally inert.
                0.0,
                now,
            );
            flights.push(FlowFlight {
                callsign: p.callsign.clone(),
                dep,
                aircraft_type: ty,
                gate: gate.clone(),
                status: "airborne".into(),
                distance_nm: Some(pred.route_nm),
                eta: Some(pred.eta),
                groundspeed: p.groundspeed,
                excluded,
                ..Default::default()
            });
            etd_ms.push(None);
        } else {
            // On the ground (or position-less): estimate a full route flight time.
            let (route_nm, ft_min) = ground_estimate(
                nav,
                &dep,
                arr,
                fp,
                airports,
                winds,
                profile,
                Some((p.latitude, p.longitude, p.heading, p.groundspeed)),
                gates,
                runways,
                taxi_samples,
                now,
            );
            flights.push(FlowFlight {
                callsign: p.callsign.clone(),
                dep,
                aircraft_type: ty,
                gate,
                status: "ground".into(),
                distance_nm: Some(route_nm),
                eta: Some(now + minutes(ft_min)),
                etd: Some(now),
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
        let profile = profiles.resolve(&ty, &wake);
        let dep = fp.departure.to_ascii_uppercase();
        let gate = arrival_gate(&fp.route, icao);
        let excluded = program.is_some_and(|pg| is_excluded(&ty, &wake, pg));
        // A prefile has no live position — gate/runway matching is skipped (`ground_estimate`
        // passes `None`), falling to the airport/default tier.
        let (route_nm, ft_min) = ground_estimate(
            nav,
            &dep,
            arr,
            fp,
            airports,
            winds,
            profile,
            None,
            gates,
            runways,
            taxi_samples,
            now,
        );
        let etd = proposed_etd(&fp.deptime, now);
        flights.push(FlowFlight {
            callsign: pf.callsign.clone(),
            dep,
            aircraft_type: ty,
            gate,
            status: "proposed".into(),
            distance_nm: Some(route_nm),
            eta: Some(etd + minutes(ft_min)),
            etd: Some(etd),
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
        if let Some(g) = &flights[i].gate
            && let Some(&lg) = last_gate_sta.get(g)
        {
            sta = sta.max(lg + gate_spacing_ms(pg, runway, g));
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
        let cand = earliest_clear_slot(eta, &assigned, |sg| match &gate {
            Some(g) if !sg.is_empty() && g == sg => gate_spacing_ms(pg, runway, g),
            _ => runway,
        });
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

/// Push `start` (epoch-ms) forward to the earliest instant clear of every occupied slot in
/// `slots` (`(time_ms, gate)`), where each slot demands `sep(gate)` ms of separation on either
/// side. One ascending pass is sufficient *and* necessary: `cand` only ever moves forward, so
/// once it clears an earlier slot it stays clear — while the `while moved { for slot … }`
/// fixpoint this replaces could **fail to terminate**. At epoch-ms magnitudes (t ≈ 1.7e12) a
/// non-integer `sep` (AAR that doesn't divide 3,600,000) makes `t + req` round to a value still
/// `< req` from `t`; the conflict never clears, so the fixpoint spins a worker at 100% CPU
/// forever (the "stuck on Loading…" outage).
fn earliest_clear_slot(start: f64, slots: &[(f64, String)], sep: impl Fn(&str) -> f64) -> f64 {
    let mut ordered: Vec<&(f64, String)> = slots.iter().collect();
    ordered.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut cand = start;
    for (t, gate) in ordered {
        let req = sep(gate);
        if (cand - t).abs() < req {
            cand = t + req;
        }
    }
    cand
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
#[allow(clippy::too_many_arguments)]
pub fn ready_time_slot(
    icao: &str,
    program: &ProgramInputs,
    data: &VatsimData,
    airports: &AirportDb,
    nav: &NavData,
    winds: &Winds,
    profiles: &trajectory::ProfileTable,
    issued: &HashMap<String, DateTime<Utc>>,
    gates: &HashMap<String, Vec<AirportGateBody>>,
    runways: &RunwayDb,
    taxi_samples: &HashMap<String, Vec<taxi_estimate::TaxiSample>>,
    callsign: &str,
    ready: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let flow = compute(
        icao,
        Some(program),
        data,
        airports,
        nav,
        winds,
        profiles,
        issued,
        gates,
        runways,
        taxi_samples,
        now,
    );
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
    let cand = earliest_clear_slot((base_ms + flight_ms) as f64, &assigned, |sg| match &gate {
        Some(g) if !sg.is_empty() && g == sg => gate_spacing_ms(program, runway, g),
        _ => runway,
    });
    DateTime::from_timestamp_millis(cand as i64 - flight_ms)
}

/// Along-route length (nm) and full flight time (min) for a ground/proposed flight: resolves the
/// filed route from the surface through the shared predictor (climb → cruise → descent + taxi),
/// the same path an airborne arrival takes. Falls back to a nominal 300 nm leg when there are no
/// departure/arrival coordinates to resolve a route from.
///
/// `pilot_pos` (`lat, lon, hdg, gs`) is the pilot's real position/heading/groundspeed when known —
/// used to resolve a gate/runway match for the learned ground allowance (#164 sub-issue E); `None`
/// for a prefile (no live position), which skips matching and falls to the airport/default tier.
#[allow(clippy::too_many_arguments)]
fn ground_estimate(
    nav: &NavData,
    dep: &str,
    arr: Option<(f64, f64)>,
    fp: &super::vatsim::FlightPlan,
    airports: &AirportDb,
    winds: &Winds,
    profile: &trajectory::AircraftProfile,
    pilot_pos: Option<(f64, f64, i64, i64)>,
    gates: &HashMap<String, Vec<AirportGateBody>>,
    runways: &RunwayDb,
    taxi_samples: &HashMap<String, Vec<taxi_estimate::TaxiSample>>,
    now: DateTime<Utc>,
) -> (f64, f64) {
    let cruise = trajectory::parse_alt_ft(&fp.altitude);
    let cruise_tas = trajectory::capped_cruise_tas(parse_tas(&fp.cruise_tas), cruise, profile);
    let aircraft = (!fp.aircraft_short.is_empty()).then_some(fp.aircraft_short.as_str());
    let allowance =
        resolve_ground_allowance_sec(gates, runways, taxi_samples, dep, aircraft, pilot_pos);
    match (airports.get(dep).copied(), arr) {
        (Some(dep_ll), Some(arr_ll)) => {
            let pred = predict::arrival_eta(
                nav,
                airports,
                winds,
                profile,
                &predict::ArrivalInput {
                    dep,
                    arr: &fp.arrival,
                    route: &fp.route,
                    pos: [dep_ll.0, dep_ll.1],
                    alt_ft: 0.0,
                    gs: 0,
                    hdg: 0,
                    arr_ll: [arr_ll.0, arr_ll.1],
                    cruise_ft: cruise,
                    cruise_tas,
                },
                allowance,
                now,
            );
            let ft_min = (pred.eta - now).num_seconds() as f64 / 60.0;
            (pred.route_nm, ft_min)
        }
        _ => {
            let vp = trajectory::VerticalProfile::build(
                0.0, 300.0, 0.0, cruise, cruise_tas, profile, None,
            );
            let ft_min = vp.time_between(300.0, 0.0) / 60.0 + allowance / 60.0;
            (300.0, ft_min)
        }
    }
}

/// Estimated departure time for a prefile: filed `deptime` (HHMM Z) nudged into a sane
/// window, else 20 minutes out. Never earlier than 20 minutes from now.
fn proposed_etd(deptime: &str, now: DateTime<Utc>) -> DateTime<Utc> {
    let floor = now + Duration::minutes(20);
    if deptime.len() == 4 && deptime.chars().all(|c| c.is_ascii_digit()) {
        let h: u32 = deptime[0..2].parse().unwrap_or(99);
        let m: u32 = deptime[2..4].parse().unwrap_or(99);
        if h < 24
            && m < 60
            && let Some(naive) = now.date_naive().and_hms_opt(h, m, 0)
        {
            let mut t = naive.and_utc();
            if t < now - Duration::hours(2) {
                t += Duration::hours(24);
            } else if t - now > Duration::hours(12) {
                t -= Duration::hours(24);
            }
            return t.max(floor);
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
    if (60.0..=1200.0).contains(&n) {
        n
    } else {
        420.0
    }
}

/// Arrival gate (STAR/fix) heuristic ported from vatflow's `arrivalGate`: scan the filed
/// route from the end and return the first token that looks like a 5-letter RNAV fix, a
/// 3-letter navaid, or a STAR/SID name (e.g. `OZZZI4`). `arr` must be uppercase.
pub fn arrival_gate(route: &str, arr: &str) -> Option<String> {
    if route.trim().is_empty() {
        return None;
    }
    // Split on whitespace *and* the procedure/field separators `.` and `/`, so dotted procedure
    // notation (`EPH.GLASR3`, `GLASR3.HAWKZ`) and speed/level suffixes (`GLASR3/N0450F350`) still
    // yield the bare STAR token instead of a stripped-together blob.
    let toks: Vec<String> = route
        .to_ascii_uppercase()
        .split([' ', '\t', '\n', '\r', '.', '/'])
        .map(|t| {
            t.chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
        })
        .filter(|t| !t.is_empty())
        .collect();
    // Prefer the named arrival procedure — a STAR is the actual arrival gate, so it wins over a
    // trailing navaid (e.g. `GLASR3 SEA` should gate on GLASR3, not the Seattle VOR).
    if let Some(star) = toks.iter().rev().find(|t| is_star(t)) {
        return Some(star.clone());
    }
    // Otherwise fall back to the last enroute fix/navaid (routes that gate on a plain fix).
    for t in toks.iter().rev() {
        if t == "DCT" || t == arr {
            continue;
        }
        if is_fix(t) || is_navaid(t) {
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

/// The gate nearest `(lat, lon)`, within [`GATE_MATCH_MAX_NM`], else `None`. Shared by
/// `feed::taxi_observations` (matching a completed departure's spawn point, #164 sub-issue C) and
/// [`resolve_ground_allowance_sec`] below (matching a live/proposed departure's position, sub-issue
/// E) — one implementation, not two.
pub(crate) fn nearest_gate(gates: &[AirportGateBody], lat: f64, lon: f64) -> Option<String> {
    let mut best: Option<(String, f64)> = None;
    for g in gates {
        let d = gc_dist(lat, lon, g.lat, g.lon);
        if best.as_ref().is_none_or(|(_, bd)| d < *bd) {
            best = Some((g.id.clone(), d));
        }
    }
    best.filter(|(_, d)| *d <= GATE_MATCH_MAX_NM)
        .map(|(id, _)| id)
}

/// The full derivation behind a ground allowance (#164 sub-issue F): which gate/runway matched (if
/// any), and the pushback/taxi estimates `taxi_estimate`'s ladder produced from them. Debug-mode
/// surfaces read this directly; [`resolve_ground_allowance_sec`] is the plain-total shorthand most
/// callers want.
pub(crate) struct GroundAllowanceBreakdown {
    pub gate_id: Option<String>,
    pub runway: Option<String>,
    pub pushback: taxi_estimate::MetricEstimate,
    pub taxi: taxi_estimate::MetricEstimate,
}

impl GroundAllowanceBreakdown {
    pub fn total_sec(&self) -> f64 {
        self.pushback.value_sec + self.taxi.value_sec
    }
}

/// The ground allowance (pushback+startup + taxi-out, #164 sub-issue E) for a departure from `dep`:
/// looks up `dep`'s cached observation samples, resolves a gate match whenever a real position
/// (`pos`) is known, resolves a *runway* match only once `pos`'s groundspeed clears
/// [`TAXI_ROLL_GS_KT`] (a stationary/gate-parked aircraft's heading is not runway-meaningful — see
/// that constant), and walks `taxi_estimate`'s fallback ladder. A prefile with no position, a route
/// with no coordinate signal, or an aircraft that hasn't started moving yet all still resolve via
/// the airport/default tier, so nothing is silently dropped. `aircraft` should be the raw
/// `FlightPlan::aircraft_short` (unnormalized), matching exactly what `feed::taxi_observations`
/// persisted, or the gate/type/runway tier will never match on type.
pub(crate) fn resolve_ground_allowance(
    gates: &HashMap<String, Vec<AirportGateBody>>,
    runways: &RunwayDb,
    taxi_samples: &HashMap<String, Vec<taxi_estimate::TaxiSample>>,
    dep: &str,
    aircraft: Option<&str>,
    pos: Option<(f64, f64, i64, i64)>,
) -> GroundAllowanceBreakdown {
    let empty: Vec<AirportGateBody> = Vec::new();
    let dep_samples = taxi_samples.get(dep).map(Vec::as_slice).unwrap_or(&[]);
    let (gate_id, runway) = match pos {
        Some((lat, lon, hdg, gs)) => (
            nearest_gate(gates.get(dep).unwrap_or(&empty), lat, lon),
            (gs > TAXI_ROLL_GS_KT)
                .then(|| nearest_runway(runways, dep, hdg))
                .flatten(),
        ),
        None => (None, None),
    };
    let est = taxi_estimate::estimate(dep_samples, gate_id.as_deref(), aircraft, runway.as_deref());
    GroundAllowanceBreakdown {
        gate_id,
        runway,
        pushback: est.pushback,
        taxi: est.taxi,
    }
}

/// Just the total seconds from [`resolve_ground_allowance`] — most callers don't need the
/// derivation, only the number `predict::eta_along_route` adds to a not-yet-airborne ETA.
pub(crate) fn resolve_ground_allowance_sec(
    gates: &HashMap<String, Vec<AirportGateBody>>,
    runways: &RunwayDb,
    taxi_samples: &HashMap<String, Vec<taxi_estimate::TaxiSample>>,
    dep: &str,
    aircraft: Option<&str>,
    pos: Option<(f64, f64, i64, i64)>,
) -> f64 {
    resolve_ground_allowance(gates, runways, taxi_samples, dep, aircraft, pos).total_sec()
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

    // The slot sweep must terminate and return a slot clear of every occupant, even with a
    // non-integer separation (AAR that doesn't divide 3.6e6) at real epoch-ms magnitudes — the
    // exact shape that made the old `while moved` fixpoint spin a worker at 100% CPU forever.
    #[test]
    fn earliest_clear_slot_terminates_on_fractional_separation() {
        let req = 3_600_000.0 / 7.0; // AAR 7 → 514285.714… ms, non-integer
        let base = 1_700_000_000_000.0; // ~epoch-ms now
        // A dense wall of occupied slots one `req` apart, forcing many forward pushes.
        let slots: Vec<(f64, String)> = (0..50)
            .map(|k| (base + k as f64 * req, String::new()))
            .collect();
        let got = earliest_clear_slot(base, &slots, |_| req);
        // Clear of every occupant by at least `req` (allowing a 1ms float slop).
        for (t, _) in &slots {
            assert!(
                (got - t).abs() >= req - 1.0,
                "slot at {t} is within {req} of result {got}"
            );
        }
        // And it landed just past the last occupant, not somewhere absurd.
        let last = base + 49.0 * req;
        assert!(
            got >= last && got <= last + 2.0 * req,
            "unexpected slot {got}"
        );
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
            ..Default::default()
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

    #[test]
    fn arrival_gate_prefers_the_star() {
        // Dotted procedure notation (transition.procedure / procedure.transition) must still resolve.
        assert_eq!(
            arrival_gate("EPH.GLASR3 KSEA", "KSEA").as_deref(),
            Some("GLASR3")
        );
        assert_eq!(
            arrival_gate("GLASR3.HAWKZ KSEA", "KSEA").as_deref(),
            Some("GLASR3")
        );
        // A trailing navaid (the Seattle VOR) must not win over the STAR.
        assert_eq!(
            arrival_gate("MWH EPH GLASR3 SEA", "KSEA").as_deref(),
            Some("GLASR3")
        );
        assert_eq!(
            arrival_gate("KGEG MWH EPH GLASR3", "KSEA").as_deref(),
            Some("GLASR3")
        );
        // Speed/level suffix on the STAR token.
        assert_eq!(
            arrival_gate("EPH GLASR3/N0450F350", "KSEA").as_deref(),
            Some("GLASR3")
        );
        // No STAR filed → still falls back to the last plain fix.
        assert_eq!(
            arrival_gate("DCT CAMRN KJFK", "KJFK").as_deref(),
            Some("CAMRN")
        );
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
                ..Default::default()
            }],
            ..Default::default()
        };
        let pg = base_program();
        let flow = compute(
            "KJFK",
            Some(&pg),
            &data,
            &airports(),
            &NavData::default(),
            &Winds::default(),
            &trajectory::ProfileTable::default(),
            &HashMap::new(),
            &HashMap::new(),
            &RunwayDb::default(),
            &HashMap::new(),
            t0(),
        );

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
                ..Default::default()
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
            &NavData::default(),
            &Winds::default(),
            &trajectory::ProfileTable::default(),
            &HashMap::new(),
            &HashMap::new(),
            &RunwayDb::default(),
            &HashMap::new(),
            "GRD1",
            ready,
            t0(),
        )
        .unwrap();
        assert!(slot >= ready, "slot {slot} should be >= ready {ready}");
    }

    /// AC #3 (cross-surface): the arrival ladder (`compute`) and the runway ETE
    /// (`runway::collect_arrivals`) must time the same aircraft identically — both route through
    /// `predict::arrival_eta` — and off the **resolved** filed route, not a straight line. The
    /// aircraft files a real NJ-coast routing (`RBV WHITE SIE`, bundled nav), so its along-route
    /// distance runs meaningfully longer than the great circle to the field; reverting
    /// `predict::arrival_eta` to `gc_dist` fails the distance assertion.
    #[test]
    fn ladder_and_runway_ete_agree_on_the_resolved_route() {
        let ap: AirportDb = HashMap::from([
            ("KJFK".to_string(), (40.6413, -73.7781)),
            ("KDCA".to_string(), (38.8521, -77.0377)),
        ]);
        let nav = NavData::load();
        let profiles = trajectory::ProfileTable::default();
        // Airborne B738 just south of KJFK tracking SW down the coast, filed KJFK -> KDCA.
        let mut p = pilot(
            "AAL1",
            40.2,
            -74.0,
            24_000,
            400,
            fp("KJFK", "KDCA", "RBV WHITE SIE"),
        );
        p.heading = 220;
        let data = VatsimData {
            pilots: vec![p],
            ..Default::default()
        };
        let flow = compute(
            "KDCA",
            None,
            &data,
            &ap,
            &nav,
            &Winds::default(),
            &profiles,
            &HashMap::new(),
            &HashMap::new(),
            &RunwayDb::default(),
            &HashMap::new(),
            t0(),
        );
        let arrivals = crate::feed::runway::collect_arrivals(
            "KDCA",
            &data,
            &ap,
            &nav,
            &Winds::default(),
            &profiles,
            t0(),
            600,
        );
        let f = flow
            .flights
            .iter()
            .find(|f| f.callsign == "AAL1")
            .expect("ladder flight");
        let ladder = f.eta.expect("ladder ETA");
        let ete = arrivals
            .iter()
            .find(|a| a.cs == "AAL1")
            .expect("runway ETE")
            .eta_ms;

        let expected_len = crate::feed::predict::path_len_nm(
            &crate::feed::fca::route_path(
                &nav,
                &ap,
                "KJFK",
                "KDCA",
                "RBV WHITE SIE",
                40.2,
                -74.0,
                220,
                400,
            )
            .expect("route resolves"),
        );
        let straight = gc_dist(40.2, -74.0, 38.8521, -77.0377);
        assert!(
            (f.distance_nm.unwrap() - expected_len).abs() < 1.0,
            "ladder distance {:?} should be the resolved route length {expected_len:.0} nm",
            f.distance_nm
        );
        assert!(
            expected_len > straight + 20.0,
            "the coastal routing {expected_len:.0} nm should exceed the {straight:.0} nm straight line"
        );
        assert!(
            (ladder.timestamp_millis() - ete).abs() < 1000,
            "ladder {ladder} and runway ETE {ete}ms disagree"
        );
    }

    fn base_program_aar(aar: i32) -> ProgramInputs {
        ProgramInputs {
            aar,
            ..base_program()
        }
    }

    fn gate(id: &str, lat: f64, lon: f64) -> AirportGateBody {
        AirportGateBody {
            id: id.to_string(),
            icao: "KAAA".to_string(),
            name: id.to_string(),
            lat,
            lon,
            source: "manual".to_string(),
            updated_at: Utc::now(),
            editable: false,
        }
    }

    #[test]
    fn nearest_gate_matches_the_closest_within_range() {
        let gates = vec![gate("A1", 40.0, -74.0), gate("A2", 40.01, -74.0)];
        assert_eq!(nearest_gate(&gates, 40.0001, -74.0), Some("A1".to_string()));
        assert_eq!(nearest_gate(&gates, 40.0099, -74.0), Some("A2".to_string()));
    }

    #[test]
    fn nearest_gate_none_when_too_far() {
        let gates = vec![gate("A1", 40.0, -74.0)];
        assert_eq!(nearest_gate(&gates, 41.0, -74.0), None);
        assert_eq!(nearest_gate(&[], 40.0, -74.0), None);
    }

    /// #164 sub-issue E, AC #3: an airport with zero taxi samples still resolves a sane, panic-free
    /// ground allowance — falling straight through `taxi_estimate`'s ladder to its flat default.
    #[test]
    fn resolve_ground_allowance_sec_defaults_for_a_thin_data_airport() {
        let gates = HashMap::new();
        let runways = RunwayDb::default();
        let samples = HashMap::new();
        let allowance = resolve_ground_allowance_sec(
            &gates,
            &runways,
            &samples,
            "KAAA",
            Some("B738"),
            Some((40.0, -74.0, 270, 20)),
        );
        assert_eq!(allowance, predict::GROUND_TAXI_SEC + 300.0);
    }

    /// No position (a prefile, or the coordinate-less `ground_estimate` fallback) skips gate/runway
    /// matching entirely rather than guessing — still resolves via the airport/default tier.
    #[test]
    fn resolve_ground_allowance_sec_skips_gate_and_runway_matching_without_a_position() {
        let gates = HashMap::from([("KAAA".to_string(), vec![gate("A1", 40.0, -74.0)])]);
        let runways = RunwayDb::default();
        let samples = HashMap::new();
        let allowance =
            resolve_ground_allowance_sec(&gates, &runways, &samples, "KAAA", Some("B738"), None);
        assert_eq!(allowance, predict::GROUND_TAXI_SEC + 300.0);
    }

    fn taxi_sample(
        gate: &str,
        aircraft: &str,
        runway: &str,
        taxi_sec: i32,
    ) -> taxi_estimate::TaxiSample {
        taxi_estimate::TaxiSample {
            gate_id: Some(gate.to_string()),
            aircraft: Some(aircraft.to_string()),
            runway: Some(runway.to_string()),
            pushback_sec: Some(50),
            taxi_sec,
        }
    }

    /// A stationary aircraft's heading is parking orientation, not runway alignment — even when it
    /// happens to point exactly down a real runway's centerline, `resolve_ground_allowance_sec`
    /// must not trust it: the gate/type/runway tier (600s) must NOT be picked over the airport-wide
    /// blend (5 samples at 600s + 5 at 120s, median 360s) while groundspeed is at or below
    /// [`TAXI_ROLL_GS_KT`].
    #[test]
    fn resolve_ground_allowance_sec_ignores_a_coincidental_heading_match_while_stationary() {
        let runways = RunwayDb::load();
        let end = runways
            .ends_for("KJFK")
            .into_iter()
            .find(|e| e.id == "04L")
            .expect("KJFK 04L is in the bundled runway data");
        let gates = HashMap::from([("KJFK".to_string(), vec![gate("A1", 40.0, -74.0)])]);
        let mut rows: Vec<taxi_estimate::TaxiSample> = (0..5)
            .map(|_| taxi_sample("A1", "B738", "04L", 600))
            .collect();
        rows.extend((0..5).map(|_| taxi_sample("B2", "A320", "22R", 120)));
        let samples = HashMap::from([("KJFK".to_string(), rows)]);

        let stationary = resolve_ground_allowance_sec(
            &gates,
            &runways,
            &samples,
            "KJFK",
            Some("B738"),
            Some((40.0, -74.0, end.hdg as i64, TAXI_ROLL_GS_KT)),
        );
        // Airport-wide blend (median of the combined 10 samples), not the runway-specific 600s.
        assert_eq!(stationary, 360.0 + 50.0);

        let rolling = resolve_ground_allowance_sec(
            &gates,
            &runways,
            &samples,
            "KJFK",
            Some("B738"),
            Some((40.0, -74.0, end.hdg as i64, TAXI_ROLL_GS_KT + 1)),
        );
        // Once actually moving, the same heading legitimately resolves the gate/type/runway tier.
        assert_eq!(rolling, 600.0 + 50.0);
    }

    /// #164 sub-issue F: debug mode reads `resolve_ground_allowance`'s breakdown directly, so it
    /// must expose the actual matched gate/runway and each metric's real tier + sample count — not
    /// just the folded-together total `resolve_ground_allowance_sec` returns.
    #[test]
    fn resolve_ground_allowance_exposes_the_matched_key_and_tier() {
        let runways = RunwayDb::load();
        let end = runways
            .ends_for("KJFK")
            .into_iter()
            .find(|e| e.id == "04L")
            .expect("KJFK 04L is in the bundled runway data");
        let gates = HashMap::from([("KJFK".to_string(), vec![gate("A1", 40.0, -74.0)])]);
        let rows: Vec<taxi_estimate::TaxiSample> = (0..5)
            .map(|_| taxi_sample("A1", "B738", "04L", 600))
            .collect();
        let samples = HashMap::from([("KJFK".to_string(), rows)]);

        let breakdown = resolve_ground_allowance(
            &gates,
            &runways,
            &samples,
            "KJFK",
            Some("B738"),
            Some((40.0, -74.0, end.hdg as i64, TAXI_ROLL_GS_KT + 1)),
        );
        assert_eq!(breakdown.gate_id.as_deref(), Some("A1"));
        assert_eq!(breakdown.runway.as_deref(), Some("04L"));
        assert_eq!(
            breakdown.taxi.tier,
            taxi_estimate::EstimateTier::GateTypeRunway
        );
        assert_eq!(breakdown.taxi.sample_count, 5);
        assert_eq!(breakdown.taxi.value_sec, 600.0);
        assert_eq!(breakdown.pushback.value_sec, 50.0);
        assert_eq!(breakdown.total_sec(), 650.0);

        // A thin-data airport falls to the default tier with no matched key at all.
        let default_breakdown = resolve_ground_allowance(
            &HashMap::new(),
            &RunwayDb::default(),
            &HashMap::new(),
            "KAAA",
            Some("B738"),
            Some((40.0, -74.0, 270, 20)),
        );
        assert_eq!(default_breakdown.gate_id, None);
        assert_eq!(default_breakdown.runway, None);
        assert_eq!(
            default_breakdown.taxi.tier,
            taxi_estimate::EstimateTier::Default
        );
    }
}
