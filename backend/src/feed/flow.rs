//! Arrival-flow computation for one airport — classifies inbound traffic (airborne /
//! ground / proposed), estimates ETAs (shared climb-profile + winds model), and meters
//! demand against a program's AAR. Ported from vatflow's `computeFlow`.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use utoipa::ToSchema;

use super::delays::nearest_runway;
use super::predict;
use super::runway_db::RunwayDb;
use super::taxi_estimate;
use super::vatsim::VatsimData;
use crate::models::AirportGateBody;

/// A gate/parking spot must be within this of a spawn point to count as a match (#164 sub-issue
/// C/E — matches `feed::taxi_observations`'s own persistence-time gate matching exactly).
const GATE_MATCH_MAX_NM: f64 = 0.06; // ~360 ft

/// Groundspeed above which a pilot's heading reflects actual taxi movement rather than arbitrary
/// gate-parking orientation (#164 sub-issue E). A stationary aircraft's heading is essentially
/// random with respect to any runway, so [`resolve_ground_allowance`] only trusts it — and
/// attempts a runway match — once the aircraft has crossed this threshold; below it, the ladder
/// falls to the airport/default tier instead of risking a coincidental wrong-runway match.
/// Mirrors `taxi_observations`'s own roll-detection threshold (`GS_START`) — the same physical
/// moment, one shared constant.
pub(crate) const TAXI_ROLL_GS_KT: i64 = 7;

/// Where an airport-flow miles-in-trail is converted to a time gap: the trailing flight's predicted
/// groundspeed this far (nm) along-route short of the field (#813). Airport-flow MIT is set per
/// arrival gate, and the gate is usually a STAR name rather than a point on the route, so there is
/// no fix to read a speed at; OIS holds no meter-fix data per airport.
pub(crate) const MIT_CROSSING_NM: f64 = 40.0;

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
    /// Wake/weight category (`L`/`M`/`H`/`J`) parsed from the filed aircraft string; null when
    /// not one of those four (AADC's "Aircraft Category" dimension, #242).
    pub category: Option<String>,
    /// Arrival gate (STAR/fix) derived from the filed route; null if none matched.
    pub gate: Option<String>,
    /// `airborne` | `ground` | `proposed` | `arrived`.
    pub status: String,
    pub distance_nm: Option<f64>,
    pub eta: Option<DateTime<Utc>>,
    /// Estimated wheels-up for ground/proposed inbounds, plus the ground allowance in each case: now
    /// for a ground flight; for a prefile, its filed departure time (never earlier than now + 20 min),
    /// or now + 20 min when it filed none or one that does not parse. Null once airborne. Used to back
    /// out enroute time for airport-flow CFRs and GDP EDCTs.
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
    // The predicted groundspeed (kt) at `MIT_CROSSING_NM` from the field, that converts this flight's
    // miles-in-trail to a time gap. Internal to metering, never serialized.
    #[serde(skip)]
    pub mit_cross_kt: f64,
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
    /// Every STAR the nav data says serves this airport (raw procedure names, e.g. `CAPPS3`),
    /// sorted, whether or not any traffic is filed on it. Lets a gate be picked before a flight
    /// files through it; group it with the same normalizer as `FlowFlight.gate`.
    pub stars: Vec<String>,
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
        if predict::is_airborne(p.groundspeed) {
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
/// maps callsign -> locked wheels-up for any CFRs already issued into this field. Every flight is
/// timed through [`predict::trajectory`], which decides its inputs (#813).
pub fn compute(
    icao: &str,
    program: Option<&ProgramInputs>,
    data: &VatsimData,
    inp: &predict::Inputs,
    issued: &HashMap<String, DateTime<Utc>>,
    // Callsigns manually dropped as bogus by any facility (#342). Airport flow has no single
    // facility context, and a flight with garbage data is garbage on every scope, so a removal
    // anywhere keeps it out of demand / AADC / metering here.
    manual_exclusions: &HashSet<String>,
    now: DateTime<Utc>,
) -> Flow {
    let arr = inp.airports.get(icao).map(|a| (a.lat, a.lon));
    let mut flights: Vec<FlowFlight> = Vec::new();
    // Estimated departure time (ms) per flight, aligned with `flights`; used to back out
    // wheels-up (CFR) from the metered STA. None for airborne/arrived (already flying).
    let mut etd_ms: Vec<Option<i64>> = Vec::new();

    let pilots = data.pilots.iter().map(predict::Flight::Pilot);
    let prefiles = data.prefiles.iter().map(predict::Flight::Prefile);
    for flight in pilots.chain(prefiles) {
        let (callsign, fp) = match flight {
            predict::Flight::Pilot(p) => (&p.callsign, p.flight_plan.as_ref()),
            predict::Flight::Prefile(pf) => (&pf.callsign, pf.flight_plan.as_ref()),
        };
        let Some(fp) = fp else { continue };
        if fp.arrival.to_ascii_uppercase() != icao {
            continue;
        }
        // A prefile that lingers after its pilot connects is the same aircraft.
        if matches!(flight, predict::Flight::Prefile(_))
            && flights.iter().any(|f| &f.callsign == callsign)
        {
            continue;
        }
        let (ty, wake) = fp.aircraft_type_wake();
        let excluded = program.is_some_and(|pg| is_excluded(&ty, &wake, pg))
            || manual_exclusions.contains(callsign);
        let mut row = FlowFlight {
            callsign: callsign.clone(),
            dep: fp.departure.to_ascii_uppercase(),
            aircraft_type: ty,
            category: (!wake.is_empty()).then_some(wake),
            gate: arrival_gate(&fp.route, icao),
            excluded,
            ..Default::default()
        };
        let etd = time_flow_flight(&mut row, flight, inp, arr, now);
        flights.push(row);
        etd_ms.push(etd);
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
        stars: inp.nav.stars_for(icao).to_vec(),
    }
}

/// Status, distance, ETA and wheels-up of one inbound, read off its [`predict::trajectory`]. Returns
/// the wheels-up (epoch ms) metering backs a CFR out of; `None` once airborne or arrived.
fn time_flow_flight(
    row: &mut FlowFlight,
    flight: predict::Flight,
    inp: &predict::Inputs,
    arr: Option<(f64, f64)>,
    now: DateTime<Utc>,
) -> Option<i64> {
    let pilot = match flight {
        predict::Flight::Pilot(p) => Some(p),
        predict::Flight::Prefile(_) => None,
    };
    let airborne = pilot.is_some_and(|p| predict::is_airborne(p.groundspeed));
    let to_field = pilot
        .zip(arr)
        .map(|(p, (alat, alon))| gc_dist(p.latitude, p.longitude, alat, alon));
    row.groundspeed = pilot.map_or(0, |p| p.groundspeed);
    if pilot.is_some() && !airborne && to_field.is_some_and(|d| d < predict::ARRIVED_NM) {
        row.status = "arrived".into();
        row.distance_nm = to_field;
        row.eta = Some(now);
        return None;
    }
    row.status = match (airborne, pilot) {
        (true, _) => "airborne",
        (false, Some(_)) => "ground",
        (false, None) => "proposed",
    }
    .into();
    let traj = predict::trajectory(inp, flight, None, now)?;
    row.distance_nm = Some(traj.route_nm());
    row.eta = Some(traj.eta_at_field());
    row.mit_cross_kt = traj.gs_short_of_field(MIT_CROSSING_NM);
    if airborne {
        return None;
    }
    // A wheels-up time, not a gate time: every CFR backed out of it is read as wheels-up (#815), so
    // the ground allowance sits before it.
    let wheels_up = traj.wheels_up();
    row.etd = Some(wheels_up);
    Some(wheels_up.timestamp_millis())
}

/// Forward window AADC buckets over (#242): fixed at 4 hours regardless of bucket size.
const AADC_WINDOW_HOURS: i64 = 4;
/// Carrier breakdowns are capped to the busiest operators in the window; everything else
/// (including non-airline callsigns) folds into `AADC_OTHER` so the response stays bounded at a
/// busy international airport.
const AADC_TOP_CARRIERS: usize = 8;
const AADC_OTHER: &str = "OTHER";

/// One bucket of forward arrival demand, broken down by every dimension AADC supports (#242):
/// status, aircraft category (wake), carrier, and arrival fix.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AadcBucket {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub total: i64,
    pub by_status: HashMap<String, i64>,
    pub by_category: HashMap<String, i64>,
    pub by_carrier: HashMap<String, i64>,
    pub by_afix: HashMap<String, i64>,
}

/// Full AADC response for one airport: bucketed demand plus the wind-favored AAR/ADR reference.
#[derive(Debug, Serialize, ToSchema)]
pub struct AadcResponse {
    pub icao: String,
    pub bucket_min: i32,
    pub aar: i32,
    pub adr: i32,
    /// The `airport_config` row the AAR/ADR came from, if any configs exist for this airport.
    pub config_id: Option<String>,
    pub buckets: Vec<AadcBucket>,
    pub generated_at: DateTime<Utc>,
}

/// Callsign's leading airline/carrier code (e.g. `AAL123` -> `AAL`), or `None` for anything that
/// doesn't look like a scheduled-carrier callsign (bare tail numbers, etc.) — the ICAO convention
/// is a 2-4 letter operator prefix ahead of a numeric flight number.
fn carrier_of(callsign: &str) -> Option<String> {
    let prefix: String = callsign
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    (prefix.len() >= 2 && prefix.len() <= 4).then_some(prefix)
}

/// Bucket `flights` into `bucket_min`-wide slots across a fixed forward window, tallying every
/// AADC dimension per bucket. Reuses the exact same landed-exclusion + demand filter as
/// `demand_60min` above (`status != "arrived" && !excluded`, `eta` inside the window) rather than
/// reinventing it — an already-landed flight must never inflate a future bucket.
pub fn bucket_aadc(flights: &[FlowFlight], now: DateTime<Utc>, bucket_min: i32) -> Vec<AadcBucket> {
    let bucket_min = bucket_min.max(1) as i64;
    let window_end = now + Duration::hours(AADC_WINDOW_HOURS);
    let n_buckets = ((AADC_WINDOW_HOURS * 60) / bucket_min).max(1) as usize;

    let demand: Vec<&FlowFlight> = flights
        .iter()
        .filter(|f| f.status != "arrived" && !f.excluded)
        .filter(|f| f.eta.is_some_and(|e| e >= now && e < window_end))
        .collect();

    // Rank carriers over the whole window first, so one bucket's small sample doesn't arbitrarily
    // decide who gets folded into "OTHER".
    let mut carrier_totals: HashMap<String, i64> = HashMap::new();
    for f in &demand {
        if let Some(c) = carrier_of(&f.callsign) {
            *carrier_totals.entry(c).or_default() += 1;
        }
    }
    let mut ranked: Vec<&String> = carrier_totals.keys().collect();
    ranked.sort_by(|a, b| carrier_totals[*b].cmp(&carrier_totals[*a]).then(a.cmp(b)));
    let top_carriers: HashSet<&str> = ranked
        .into_iter()
        .take(AADC_TOP_CARRIERS)
        .map(|s| s.as_str())
        .collect();

    let mut buckets: Vec<AadcBucket> = (0..n_buckets)
        .map(|i| {
            let start = now + Duration::minutes(i as i64 * bucket_min);
            AadcBucket {
                end: start + Duration::minutes(bucket_min),
                start,
                total: 0,
                by_status: HashMap::new(),
                by_category: HashMap::new(),
                by_carrier: HashMap::new(),
                by_afix: HashMap::new(),
            }
        })
        .collect();

    for f in &demand {
        let Some(eta) = f.eta else { continue };
        let idx = ((eta - now).num_seconds() / (bucket_min * 60)) as usize;
        let Some(b) = buckets.get_mut(idx) else {
            continue;
        };
        b.total += 1;
        *b.by_status.entry(f.status.clone()).or_default() += 1;
        let category = f.category.clone().unwrap_or_else(|| AADC_OTHER.to_string());
        *b.by_category.entry(category).or_default() += 1;
        let carrier = carrier_of(&f.callsign)
            .filter(|c| top_carriers.contains(c.as_str()))
            .unwrap_or_else(|| AADC_OTHER.to_string());
        *b.by_carrier.entry(carrier).or_default() += 1;
        let afix = f.gate.clone().unwrap_or_else(|| AADC_OTHER.to_string());
        *b.by_afix.entry(afix).or_default() += 1;
    }

    buckets
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

/// Same-route in-trail interval (ms) from a spacing rule: MIT converted at the trailing flight's
/// predicted crossing speed `cross_kt` (floored at [`predict::MIN_CROSS_SPEED_KT`], as FCA metering
/// floors it, #355) if set, else minutes-in-trail — never below the runway interval.
fn route_interval_ms(trail: i32, mit: i32, runway_ms: f64, cross_kt: f64) -> f64 {
    let ri = if mit > 0 {
        (mit as f64 / cross_kt.max(predict::MIN_CROSS_SPEED_KT)) * 3_600_000.0
    } else {
        trail as f64 * 60_000.0
    };
    runway_ms.max(ri)
}

/// In-trail interval (ms) for a specific arrival gate: a matching per-gate rule wins,
/// else the program-wide trail/MIT, never below the runway interval.
/// `cross_kt` is the trailing flight's `FlowFlight::mit_cross_kt`.
fn gate_spacing_ms(pg: &ProgramInputs, runway_ms: f64, gate: &str, cross_kt: f64) -> f64 {
    if let Some(g) = pg.gates.iter().find(|g| g.name == gate) {
        return route_interval_ms(g.trail, g.mit, runway_ms, cross_kt);
    }
    route_interval_ms(pg.trail, pg.mit, runway_ms, cross_kt)
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

    let mut assigned: Vec<Slot> = Vec::new();
    let mut prev_sta = f64::NEG_INFINITY;
    let mut last_gate_sta: HashMap<String, f64> = HashMap::new();

    // Tier 1 — airborne fly their ETA, delayed only by runway + same-gate in-trail.
    for &i in &airborne {
        let eta = eta_ms(&flights[i], now_ms) as f64;
        let mut sta = eta.max(prev_sta + runway);
        if let Some(g) = &flights[i].gate
            && let Some(&lg) = last_gate_sta.get(g)
        {
            sta = sta.max(lg + gate_spacing_ms(pg, runway, g, flights[i].mit_cross_kt));
        }
        write_meter(&mut flights[i], sta, eta, etd_ms[i]);
        prev_sta = sta;
        if let Some(g) = flights[i].gate.clone() {
            last_gate_sta.insert(g, sta);
        }
        assigned.push(Slot::of(&flights[i], sta));
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
        assigned.push(Slot::of(&flights[i], sta as f64));
    }

    // Tier 2b — everything else on the ground slots into the first gap clear of every slot.
    for &i in &ground {
        let eta = eta_ms(&flights[i], now_ms) as f64;
        let gate = flights[i].gate.clone();
        let cand = earliest_clear_slot(
            eta,
            flights[i].mit_cross_kt,
            &assigned,
            |sg, kt| match &gate {
                Some(g) if !sg.is_empty() && g == sg => gate_spacing_ms(pg, runway, g, kt),
                _ => runway,
            },
        );
        write_meter(&mut flights[i], cand, eta, etd_ms[i]);
        assigned.push(Slot::of(&flights[i], cand));
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

/// An occupied arrival slot: its time (epoch-ms), gate (empty for none) and the holder's
/// `FlowFlight::mit_cross_kt`.
struct Slot {
    t: f64,
    gate: String,
    cross_kt: f64,
}

impl Slot {
    fn of(f: &FlowFlight, t: f64) -> Self {
        Slot {
            t,
            gate: f.gate.clone().unwrap_or_default(),
            cross_kt: f.mit_cross_kt,
        }
    }
}

/// Push `start` (epoch-ms) forward to the earliest instant clear of every occupied slot in
/// `slots`. `sep(gate, kt)` is the separation behind a leader on `gate` when the trailing flight
/// crosses at `kt`: the slot holder's speed while the candidate is ahead of it, `cand_kt` once
/// the candidate is behind it, which is where a conflict always moves it.
///
/// One ascending pass is sufficient *and* necessary: `cand` only ever moves forward, so
/// once it clears an earlier slot it stays clear — while the `while moved { for slot … }`
/// fixpoint this replaces could **fail to terminate**. At epoch-ms magnitudes (t ≈ 1.7e12) a
/// non-integer `sep` (AAR that doesn't divide 3,600,000) makes `t + req` round to a value still
/// `< req` from `t`; the conflict never clears, so the fixpoint spins a worker at 100% CPU
/// forever (the "stuck on Loading…" outage).
fn earliest_clear_slot(
    start: f64,
    cand_kt: f64,
    slots: &[Slot],
    sep: impl Fn(&str, f64) -> f64,
) -> f64 {
    let mut ordered: Vec<&Slot> = slots.iter().collect();
    ordered.sort_by(|a, b| a.t.total_cmp(&b.t));
    let mut cand = start;
    for slot in ordered {
        let behind = sep(&slot.gate, cand_kt);
        let conflict = if cand >= slot.t {
            cand - slot.t < behind
        } else {
            slot.t - cand < sep(&slot.gate, slot.cross_kt)
        };
        if conflict {
            cand = slot.t + behind;
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
    inp: &predict::Inputs,
    issued: &HashMap<String, DateTime<Utc>>,
    // Threaded through so the slot search sees the same flow picture as every other surface —
    // a manually excluded bogus flight (#342) must not block a real CFR slot.
    manual_exclusions: &HashSet<String>,
    callsign: &str,
    ready: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let flow = compute(
        icao,
        Some(program),
        data,
        inp,
        issued,
        manual_exclusions,
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
    let assigned: Vec<Slot> = flow
        .flights
        .iter()
        .filter(|f| f.callsign != callsign && !f.excluded && f.status != "arrived")
        .filter_map(|f| Some(Slot::of(f, f.sta?.timestamp_millis() as f64)))
        .collect();

    // Slot the flight's arrival from (ready + enroute) forward past any conflicts. Never
    // earlier than now — a release can't be issued in the past.
    let base_ms = ready.timestamp_millis().max(now.timestamp_millis());
    let start = (base_ms + flight_ms) as f64;
    let cand = earliest_clear_slot(
        start,
        target.mit_cross_kt,
        &assigned,
        |sg, kt| match &gate {
            Some(g) if !sg.is_empty() && g == sg => gate_spacing_ms(program, runway, g, kt),
            _ => runway,
        },
    );
    DateTime::from_timestamp_millis(cand as i64 - flight_ms)
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
/// [`resolve_ground_allowance`] below (matching a live/proposed departure's position, sub-issue
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
/// any), and the pushback/start-up/taxi estimates `taxi_estimate`'s ladder produced from them. Debug-mode
/// surfaces read this directly; `predict::Trajectory` reads its total.
pub(crate) struct GroundAllowanceBreakdown {
    pub gate_id: Option<String>,
    pub runway: Option<String>,
    pub pushback: taxi_estimate::MetricEstimate,
    pub startup: taxi_estimate::MetricEstimate,
    pub taxi: taxi_estimate::MetricEstimate,
}

impl GroundAllowanceBreakdown {
    pub fn total_sec(&self) -> f64 {
        self.pushback.value_sec + self.startup.value_sec + self.taxi.value_sec
    }
}

/// The ground allowance (pushback + start-up + taxi-out, #164 sub-issue E) for a departure from `dep`:
/// looks up `dep`'s cached observation samples, resolves a gate match whenever a real position
/// (`pos`) is known, resolves a *runway* match only once `pos`'s groundspeed clears
/// [`TAXI_ROLL_GS_KT`] (a stationary/gate-parked aircraft's heading is not runway-meaningful — see
/// that constant), and walks `taxi_estimate`'s fallback ladder. A prefile with no position, a route
/// with no coordinate signal, or an aircraft that hasn't started moving yet all still resolve via
/// the airport/default tier, so nothing is silently dropped. `aircraft` should be the raw
/// `FlightPlan::aircraft_short` (unnormalized), matching exactly what `feed::taxi_observations`
/// persisted, or the gate/type/runway tier will never match on type.
///
/// # `assigned` replaces the guess, it does not join it (#513)
///
/// An `assigned` runway is one somebody or something actually decided — a controller's override, a
/// facility rule, or the active configuration, via #511's ladder. It **replaces** the heading inference
/// outright rather than being weighed against it, because the two are not comparable evidence: the guess
/// answers "which runway is this aircraft pointed at *right now*", which is only meaningful mid-taxi and
/// is deliberately refused while parked. A decision outranks a geometric coincidence at any groundspeed.
///
/// This is what lets a **parked or prefiled** departure reach the gate/type/runway and airport/runway
/// tiers at all. Those aircraft have no usable heading by definition, so before #513 every one of them
/// fell to the airport/default tier — and this allowance sets the earliest wheels-up (now plus the
/// allowance), so the tier it lands in moves the ETA and the earliest EDCT a controller sees.
///
/// `None` leaves the behaviour exactly as it was: the heading guess, still gated on [`TAXI_ROLL_GS_KT`].
pub(crate) fn resolve_ground_allowance(
    gates: &HashMap<String, Vec<AirportGateBody>>,
    runways: &RunwayDb,
    taxi_samples: &HashMap<String, Vec<taxi_estimate::TaxiSample>>,
    dep: &str,
    aircraft: Option<&str>,
    pos: Option<(f64, f64, i64, i64)>,
    assigned: Option<&str>,
) -> GroundAllowanceBreakdown {
    let empty: Vec<AirportGateBody> = Vec::new();
    let dep_samples = taxi_samples.get(dep).map(Vec::as_slice).unwrap_or(&[]);
    let (gate_id, guessed) = match pos {
        Some((lat, lon, hdg, gs)) => (
            nearest_gate(gates.get(dep).unwrap_or(&empty), lat, lon),
            (gs > TAXI_ROLL_GS_KT)
                .then(|| nearest_runway(runways, dep, hdg))
                .flatten(),
        ),
        None => (None, None),
    };
    // A decision beats an inference, so `assigned` is tried first and the heading guess is only the
    // fallback — never the other way round (#513).
    let runway = assigned
        .map(|r| r.trim().to_ascii_uppercase())
        .filter(|r| !r.is_empty())
        .or(guessed);
    let est = taxi_estimate::estimate(dep_samples, gate_id.as_deref(), aircraft, runway.as_deref());
    GroundAllowanceBreakdown {
        gate_id,
        runway,
        pushback: est.pushback,
        startup: est.startup,
        taxi: est.taxi,
    }
}

/// Just the total seconds from [`resolve_ground_allowance`], for the tests that pin the allowance.
#[cfg(test)]
pub(crate) fn resolve_ground_allowance_sec(
    gates: &HashMap<String, Vec<AirportGateBody>>,
    runways: &RunwayDb,
    taxi_samples: &HashMap<String, Vec<taxi_estimate::TaxiSample>>,
    dep: &str,
    aircraft: Option<&str>,
    pos: Option<(f64, f64, i64, i64)>,
    assigned: Option<&str>,
) -> f64 {
    resolve_ground_allowance(gates, runways, taxi_samples, dep, aircraft, pos, assigned).total_sec()
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
    use crate::feed::airports::{Airport, AirportDb};
    use crate::feed::nav::NavData;
    use crate::feed::trajectory;
    use crate::feed::vatsim::{FlightPlan, Pilot, Prefile, VatsimData};
    use crate::feed::winds::Winds;

    /// [`compute`] over loose tables and no assigned runways, the shape these tests were written in.
    #[allow(clippy::too_many_arguments)] // one argument per table `predict::Inputs` groups
    fn compute_with(
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
        manual_exclusions: &HashSet<String>,
        now: DateTime<Utc>,
    ) -> Flow {
        let assigned = predict::AssignedRunways::new();
        let inp = predict::Inputs {
            nav,
            airports,
            winds,
            profiles,
            ground: predict::GroundInputs {
                gates,
                runways,
                taxi_samples,
                assigned: &assigned,
            },
        };
        compute(icao, program, data, &inp, issued, manual_exclusions, now)
    }

    /// Inputs with no gate catalog, runway table or assignments, and these taxi samples.
    fn inputs_with<'a>(
        airports: &'a AirportDb,
        nav: &'a NavData,
        winds: &'a Winds,
        profiles: &'a trajectory::ProfileTable,
        taxi_samples: &'a HashMap<String, Vec<taxi_estimate::TaxiSample>>,
    ) -> predict::Inputs<'a> {
        predict::Inputs {
            nav,
            airports,
            winds,
            profiles,
            ground: predict::GroundInputs {
                taxi_samples,
                ..predict::GroundInputs::unused()
            },
        }
    }

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
        let slots: Vec<Slot> = (0..50)
            .map(|k| Slot {
                t: base + k as f64 * req,
                gate: String::new(),
                cross_kt: 300.0,
            })
            .collect();
        let got = earliest_clear_slot(base, 300.0, &slots, |_, _| req);
        // Clear of every occupant by at least `req` (allowing a 1ms float slop).
        for Slot { t, .. } in &slots {
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
            ("KJFK".to_string(), Airport::at(40.6413, -73.7781)),
            ("KBOS".to_string(), Airport::at(42.3656, -71.0096)),
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

    // ---- static gate list (#791) ----

    /// The shared gate-name fixture. `web/src/features/dashboard/tgui-gates.dom.test.tsx` reads the same
    /// file.
    fn gate_fixture() -> serde_json::Value {
        serde_json::from_str(include_str!("../../../fixtures/tgui-gate-names.json"))
            .expect("the TGUI gate-name fixture should parse")
    }

    fn gate_fixture_nav(fixture: &serde_json::Value) -> NavData {
        let procedures = fixture["procedures"].to_string();
        NavData::from_json("{}", "{}", "{}", &procedures, "{}", "{}", "{}")
    }

    fn strings(v: &serde_json::Value) -> Vec<String> {
        v.as_array()
            .expect("an array")
            .iter()
            .map(|s| s.as_str().expect("a string").to_string())
            .collect()
    }

    /// With no traffic at all, the flow still lists the airport's STARs, taken from the nav data's
    /// `apt` field: the other airport's STAR and the airport's own SID are left out.
    #[test]
    fn flow_lists_the_airports_stars_from_nav_data_with_no_traffic() {
        let fixture = gate_fixture();
        let icao = fixture["airport"].as_str().unwrap();
        let flow = compute_with(
            icao,
            None,
            &VatsimData::default(),
            &airports(),
            &gate_fixture_nav(&fixture),
            &Winds::default(),
            &trajectory::ProfileTable::default(),
            &HashMap::new(),
            &HashMap::new(),
            &RunwayDb::default(),
            &HashMap::new(),
            &HashSet::new(),
            t0(),
        );

        assert!(flow.flights.is_empty());
        assert_eq!(
            flow.stars,
            vec!["CAVLR", "CAVLR6", "DELRO5", "SEG6", "WIGOL3"],
            "KIAD's STARs, sorted, once each (CAVLR6 lists `kiad` and `KIAD`, SEG6 only `kiad`); not KDCA's FRDMM5 and \
             not the JCOBY4 SID"
        );
        assert_eq!(flow.stars, strings(&fixture["stars"]));
        // The excluded STAR is reachable for its own airport, so the filter is the airport, not
        // a parse failure.
        assert_eq!(gate_fixture_nav(&fixture).stars_for("kdca"), ["FRDMM5"]);
        assert!(gate_fixture_nav(&fixture).stars_for("KBOS").is_empty());
    }

    /// The backend half of the shared fixture: `arrival_gate` emits each case's `gate` and the nav
    /// data emits its `star`. The web test proves both strings land in the same column.
    #[test]
    fn arrival_gate_and_static_stars_emit_the_fixture_names() {
        let fixture = gate_fixture();
        let icao = fixture["airport"].as_str().unwrap();
        let nav = gate_fixture_nav(&fixture);
        let cases = fixture["cases"].as_array().unwrap();
        assert!(!cases.is_empty());
        for c in cases {
            let name = c["name"].as_str().unwrap();
            assert_eq!(
                arrival_gate(c["route"].as_str().unwrap(), icao).as_deref(),
                c["gate"].as_str(),
                "{name}: live gate"
            );
            let star = c["star"].as_str().unwrap();
            assert!(
                nav.stars_for(icao).iter().any(|s| s == star),
                "{name}: {star} should be in the static list"
            );
        }
    }

    /// The bundled nav data really carries `apt`; the fixture alone can't prove the shipped file
    /// does.
    #[test]
    fn bundled_nav_data_lists_kiad_stars() {
        let nav = NavData::load();
        let stars = nav.stars_for("KIAD");
        assert!(stars.iter().any(|s| s == "CAVLR6"), "{stars:?}");
        assert!(
            !stars.iter().any(|s| s == "LUCIT3"),
            "KIGQ's STAR leaked in"
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

    // ---- spacing math ----

    /// #813 AC4: an airport-flow MIT is a time gap at the trailing flight's predicted crossing speed:
    /// at 250 kt, 20 MIT is 20 / 250 h = 288 s, so the gap times 250 kt is the 20 nm.
    #[test]
    fn route_interval_converts_mit_at_the_crossing_speed() {
        let runway = 60_000.0; // 1 min
        let gap = route_interval_ms(0, 20, runway, 250.0);
        assert_eq!(gap, 288_000.0);
        assert_eq!(gap / 3_600_000.0 * 250.0, 20.0, "gap x 250 kt is the MIT");
        // A slower crossing opens a wider gap for the same miles.
        assert_eq!(route_interval_ms(0, 20, runway, 200.0), 360_000.0);
        // 5 minutes-in-trail = 300 s, whatever the speed.
        assert_eq!(route_interval_ms(5, 0, runway, 250.0), 300_000.0);
        // MIT overrides minutes when both set.
        assert_eq!(route_interval_ms(5, 20, runway, 250.0), 288_000.0);
        // Never below the runway interval.
        assert_eq!(route_interval_ms(0, 1, 600_000.0, 250.0), 600_000.0);
    }

    /// The crossing speed is floored at `MIN_CROSS_SPEED_KT` (120 kt), as FCA metering floors it, so
    /// a slow sample cannot open an hour-long gap: 20 MIT at 120 kt is 600 s.
    #[test]
    fn route_interval_floors_the_crossing_speed_at_120_kt() {
        assert_eq!(predict::MIN_CROSS_SPEED_KT, 120.0);
        assert_eq!(route_interval_ms(0, 20, 60_000.0, 60.0), 600_000.0);
        assert_eq!(route_interval_ms(0, 20, 60_000.0, 0.0), 600_000.0);
        assert_eq!(
            route_interval_ms(0, 20, 60_000.0, 121.0),
            20.0 / 121.0 * 3_600_000.0
        );
    }

    #[test]
    fn gate_spacing_prefers_matching_gate_rule() {
        let runway = 60_000.0;
        let pg = ProgramInputs {
            mit: 15, // airport-wide 15 MIT = 216 s at 250 kt, above the runway floor
            gates: vec![GateSpacing {
                name: "CAMRN".into(),
                trail: 0,
                mit: 20,
            }],
            ..base_program()
        };
        // Matching gate rule (20 MIT) wins.
        assert_eq!(gate_spacing_ms(&pg, runway, "CAMRN", 250.0), 288_000.0);
        // No matching rule -> airport-wide mit (15 nm).
        assert_eq!(gate_spacing_ms(&pg, runway, "LENDY", 250.0), 216_000.0);
    }

    #[test]
    fn proposed_etd_uses_deptime_or_floor() {
        use chrono::Timelike;
        // A filed deptime well ahead of now is honored.
        let etd = predict::proposed_etd("2330", t0());
        assert_eq!((etd.hour(), etd.minute()), (23, 30));
        // Blank deptime -> now + 20 minutes.
        let etd = predict::proposed_etd("", t0());
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
        let flow = compute_with(
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
            &HashSet::new(),
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
                // A 55 kt roll at field level has departed; 45 kt is still a taxi (50 kt, #813).
                pilot("ROLL", 42.36, -71.0, 0, 55, fp("KBOS", "KJFK", "DCT")),
                pilot("TAXI", 42.36, -71.0, 0, 45, fp("KBOS", "KJFK", "DCT")),
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
        assert!(!names.contains(&"ROLL"));
        assert!(names.contains(&"TAXI"));
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

    /// #815 AC1 (airport-flow path): with a ground flight's ETD at wheels-up, the CFR is the STA less
    /// the airborne ETE. `write_meter` itself is unchanged by #815; the guard against the gate-time ETD
    /// is `a_ground_flights_etd_is_its_wheels_up_estimate`.
    #[test]
    fn write_meter_cfr_is_the_sta_less_the_airborne_ete() {
        use chrono::TimeZone;
        let at = |h, m| Utc.with_ymd_and_hms(2026, 10, 10, h, m, 0).unwrap();
        let mut f = FlowFlight {
            callsign: "GRD".into(),
            status: "ground".into(),
            ..Default::default()
        };
        // Wheels-up 13:11 (now 13:00 plus an 11 min allowance), 30 min airborne: ETA 13:41.
        write_meter(
            &mut f,
            at(14, 0).timestamp_millis() as f64,
            at(13, 41).timestamp_millis() as f64,
            Some(at(13, 11).timestamp_millis()),
        );
        assert_eq!(f.sta, Some(at(14, 0)));
        assert_eq!(f.cfr, Some(at(13, 30)), "not 13:19, the gate time");
        assert_eq!(f.delay_min, 19);
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
        // Both predicted to cross 40 nm out at 360 kt: 20 MIT is 200 s.
        let mut flights = vec![
            FlowFlight {
                mit_cross_kt: 360.0,
                ..ff("A1", "airborne", 1, Some("CAMRN"))
            },
            FlowFlight {
                mit_cross_kt: 360.0,
                ..ff("A2", "airborne", 2, Some("CAMRN"))
            },
        ];
        let etd = vec![None, None];
        apply_metering(&mut flights, &etd, &pg, &HashMap::new(), now);
        // The gate in-trail (200s) exceeds the runway interval.
        assert_eq!(ms(flights[1].sta) - ms(flights[0].sta), 200_000);
        // The trailing flight's own speed sizes the gap: at 240 kt the same 20 nm is 300 s.
        flights[1].mit_cross_kt = 240.0;
        apply_metering(&mut flights, &etd, &pg, &HashMap::new(), now);
        assert_eq!(ms(flights[1].sta) - ms(flights[0].sta), 300_000);
    }

    /// A fast ground flight that would land ahead of a slow airborne one on the same gate is
    /// spaced by the slow flight's speed, since the slow one would trail it: 20 nm at 150 kt is
    /// 480 s, more than the 300 s between them, so the ground flight goes behind instead, 180 s
    /// back (20 nm at its own 400 kt).
    #[test]
    fn a_gap_ahead_of_a_slot_is_sized_at_the_slot_holders_speed() {
        let pg = ProgramInputs {
            aar: 60,
            gates: vec![GateSpacing {
                name: "CAMRN".into(),
                trail: 0,
                mit: 20,
            }],
            ..base_program()
        };
        let mut flights = vec![
            FlowFlight {
                mit_cross_kt: 150.0,
                ..ff("SLOW", "airborne", 10, Some("CAMRN"))
            },
            FlowFlight {
                mit_cross_kt: 400.0,
                ..ff("FAST", "ground", 5, Some("CAMRN"))
            },
        ];
        let etd = vec![None, Some(t0().timestamp_millis())];
        apply_metering(&mut flights, &etd, &pg, &HashMap::new(), t0());
        assert_eq!(ms(flights[0].sta), ms(Some(t0() + Duration::minutes(10))));
        assert_eq!(ms(flights[1].sta) - ms(flights[0].sta), 180_000);

        // Positive control: at 300 s ahead a 400 kt leader needs only 180 s, so a holder that
        // also crosses at 400 kt leaves the fast flight on its own ETA.
        flights[0].mit_cross_kt = 400.0;
        apply_metering(&mut flights, &etd, &pg, &HashMap::new(), t0());
        assert_eq!(ms(flights[1].sta), ms(Some(t0() + Duration::minutes(5))));
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

    /// #815 (Tier 2a): an issued CFR reserves its slot at the locked wheels-up plus the airborne
    /// ETE. The ground flight's own wheels-up estimate is now + 11 min (the 660 s allowance), so a
    /// wheels-up locked at now + 30 min lands it 19 min after its ETA. Backing out the gate time
    /// (`eta − now`) would reserve it 30 min late instead.
    #[test]
    fn an_issued_cfr_reserves_its_wheels_up_plus_the_airborne_ete() {
        let now = t0();
        // Five samples at KBOS; no gate list and a parked pilot, so the airport tier: 50 + 40 + 570 s.
        let samples = HashMap::from([(
            "KBOS".to_string(),
            (0..5)
                .map(|_| taxi_sample("A1", "B738", "04L", 570))
                .collect::<Vec<_>>(),
        )]);
        let data = VatsimData {
            pilots: vec![pilot("GRD1", 42.36, -71.0, 0, 0, fp("KBOS", "KJFK", "DCT"))],
            ..Default::default()
        };
        let wheels = now + Duration::minutes(30);
        let issued = HashMap::from([("GRD1".to_string(), wheels)]);
        let flow = compute_with(
            "KJFK",
            Some(&base_program()),
            &data,
            &airports(),
            &NavData::default(),
            &Winds::default(),
            &trajectory::ProfileTable::default(),
            &issued,
            &HashMap::new(),
            &RunwayDb::default(),
            &samples,
            &HashSet::new(),
            now,
        );
        let grd = &flow.flights[0];
        assert_eq!(
            grd.etd,
            Some(now + Duration::seconds(660)),
            "positive control"
        );
        assert!(grd.cfr_issued);
        assert_eq!(grd.cfr, Some(wheels));
        assert_eq!(
            ms(grd.sta),
            ms(grd.eta) + 19 * 60_000,
            "not 30 min, the gate-time form"
        );
        assert_eq!(grd.delay_min, 19);
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
            &inputs_with(
                &airports(),
                &NavData::default(),
                &Winds::default(),
                &trajectory::ProfileTable::default(),
                &HashMap::new(),
            ),
            &HashMap::new(),
            &HashSet::new(),
            "GRD1",
            ready,
            t0(),
        )
        .unwrap();
        assert!(slot >= ready, "slot {slot} should be >= ready {ready}");
    }

    /// #815: `ready_time_slot` returns a wheels-up, so the enroute time it slots with is the
    /// airborne ETE alone (`sta − cfr`), with no ground allowance in it. AIR is airborne, so its slot
    /// does not depend on any ETD. GRD1 is ready one airborne ETE less 1 min before AIR's slot, so it
    /// would arrive 1 min behind AIR, inside the 2 min runway interval (AAR 30), and its wheels-up is
    /// pushed 1 min later. With the 660 s allowance counted in the enroute time, the ready time would
    /// read as 12 min clear of AIR and come back unchanged.
    #[test]
    fn ready_time_slot_pushes_a_conflicting_wheels_up_by_the_runway_interval() {
        let now = t0();
        // Five samples at KBOS; no gate list and a parked pilot, so the airport tier: 50 + 40 + 570 s.
        let samples = HashMap::from([(
            "KBOS".to_string(),
            (0..5)
                .map(|_| taxi_sample("A1", "B738", "04L", 570))
                .collect::<Vec<_>>(),
        )]);
        let plan = fp("KBOS", "KJFK", "DCT");
        let data = VatsimData {
            pilots: vec![
                pilot("GRD1", 42.36, -71.0, 0, 0, plan.clone()),
                // Airborne over Chicago, far enough out that GRD1's ready time is after now.
                pilot("AIR", 41.9, -87.9, 35_000, 450, fp("KORD", "KJFK", "DCT")),
            ],
            ..Default::default()
        };
        let flow = compute_with(
            "KJFK",
            Some(&base_program()),
            &data,
            &airports(),
            &NavData::default(),
            &Winds::default(),
            &trajectory::ProfileTable::default(),
            &HashMap::new(),
            &HashMap::new(),
            &RunwayDb::default(),
            &samples,
            &HashSet::new(),
            now,
        );
        let air = flow.flights.iter().find(|f| f.callsign == "AIR").unwrap();
        assert_eq!(
            air.status, "airborne",
            "positive control: AIR's slot ignores any ETD"
        );
        let air_sta = air.sta.unwrap();
        // GRD1's trajectory from the same inputs; its airborne ETE is its ETA less its wheels-up.
        let (ap, nav, winds, profiles) = (
            airports(),
            NavData::default(),
            Winds::default(),
            trajectory::ProfileTable::default(),
        );
        let inp = inputs_with(&ap, &nav, &winds, &profiles, &samples);
        let grd =
            predict::trajectory(&inp, predict::Flight::Pilot(&data.pilots[0]), None, now).unwrap();
        assert_eq!(
            grd.allowance_sec(),
            660.0,
            "positive control: the samples were read"
        );
        let airborne_ete = grd.eta_at_field() - grd.wheels_up();
        let ready = air_sta - airborne_ete + Duration::minutes(1);
        assert!(ready > now, "ready {ready} must be after now {now}");

        let slot = ready_time_slot(
            "KJFK",
            &base_program(),
            &data,
            &inputs_with(
                &airports(),
                &NavData::default(),
                &Winds::default(),
                &trajectory::ProfileTable::default(),
                &samples,
            ),
            &HashMap::new(),
            &HashSet::new(),
            "GRD1",
            ready,
            now,
        );
        assert_eq!(slot, Some(ready + Duration::minutes(1)));
    }

    /// A CFR behind an airborne flight on the same gate is spaced by the gate's MIT at the CFR
    /// flight's own predicted crossing speed (`FlowFlight::mit_cross_kt`), the trailing flight's.
    #[test]
    fn ready_time_slot_spaces_a_gate_mit_at_the_cfr_flights_speed() {
        let now = t0();
        let pg = ProgramInputs {
            aar: 60,
            gates: vec![GateSpacing {
                name: "CAMRN".into(),
                trail: 0,
                mit: 20,
            }],
            ..base_program()
        };
        let data = VatsimData {
            pilots: vec![
                pilot("GRD1", 42.36, -71.0, 0, 0, fp("KBOS", "KJFK", "CAMRN")),
                pilot("AIR", 41.9, -87.9, 35_000, 450, fp("KORD", "KJFK", "CAMRN")),
            ],
            ..Default::default()
        };
        let (ap, nav, winds, profiles) = (
            airports(),
            NavData::default(),
            Winds::default(),
            trajectory::ProfileTable::default(),
        );
        let no_samples = HashMap::new();
        let inp = inputs_with(&ap, &nav, &winds, &profiles, &no_samples);
        let flow = compute(
            "KJFK",
            Some(&pg),
            &data,
            &inp,
            &HashMap::new(),
            &HashSet::new(),
            now,
        );
        let row = |cs: &str| flow.flights.iter().find(|f| f.callsign == cs).unwrap();
        let (air, grd) = (row("AIR"), row("GRD1"));
        assert_eq!(air.gate.as_deref(), Some("CAMRN"), "positive control");
        assert!(
            grd.mit_cross_kt > 0.0,
            "positive control: GRD1 has a crossing speed"
        );
        let traj =
            predict::trajectory(&inp, predict::Flight::Pilot(&data.pilots[0]), None, now).unwrap();
        let airborne_ete = traj.eta_at_field() - traj.wheels_up();
        let air_sta = air.sta.unwrap();
        // GRD1 is ready to land 1 min behind AIR, inside 20 nm at any speed under 1,200 kt.
        let ready = air_sta - airborne_ete + Duration::minutes(1);
        assert!(ready > now);
        let slot = ready_time_slot(
            "KJFK",
            &pg,
            &data,
            &inp,
            &HashMap::new(),
            &HashSet::new(),
            "GRD1",
            ready,
            now,
        )
        .unwrap();
        let gap_ms =
            (20.0 / grd.mit_cross_kt.max(predict::MIN_CROSS_SPEED_KT) * 3_600_000.0) as i64;
        assert!(
            gap_ms > 60_000,
            "the MIT gap is wider than the runway interval"
        );
        assert_eq!(
            ((slot + airborne_ete) - air_sta).num_milliseconds(),
            gap_ms,
            "behind AIR by 20 nm at GRD1's speed"
        );
    }

    /// #815 AC1 wiring: `compute` puts a ground flight's and a prefile's ETD at its estimated
    /// wheels-up (now, or the filed departure time, plus the ground allowance), leaves its ETA alone,
    /// and meters it with a CFR that backs out only the airborne ETE (`sta − cfr`).
    #[test]
    fn a_ground_flights_etd_is_its_wheels_up_estimate() {
        use chrono::TimeZone;
        let now = t0(); // 2023-11-14 22:13:20Z
        // Five samples at KBOS; no gate list and a parked pilot, so the airport tier: 50 + 40 + 570 s.
        let samples = HashMap::from([(
            "KBOS".to_string(),
            (0..5)
                .map(|_| taxi_sample("A1", "B738", "04L", 570))
                .collect::<Vec<_>>(),
        )]);
        let plan = fp("KBOS", "KJFK", "DCT");
        let data = VatsimData {
            pilots: vec![pilot("GRD1", 42.36, -71.0, 0, 0, plan.clone())],
            prefiles: vec![Prefile {
                callsign: "PRE1".into(),
                flight_plan: Some(FlightPlan {
                    deptime: "2300".into(),
                    ..plan.clone()
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        let flow = compute_with(
            "KJFK",
            Some(&base_program()),
            &data,
            &airports(),
            &NavData::default(),
            &Winds::default(),
            &trajectory::ProfileTable::default(),
            &HashMap::new(),
            &HashMap::new(),
            &RunwayDb::default(),
            &samples,
            &HashSet::new(),
            now,
        );
        // Each flight's trajectory, allowance included, from the same inputs: the pilot from where
        // it sits, the prefile from its departure field.
        let (ap, nav, winds, profiles) = (
            airports(),
            NavData::default(),
            Winds::default(),
            trajectory::ProfileTable::default(),
        );
        let inp = inputs_with(&ap, &nav, &winds, &profiles, &samples);
        let grd_t =
            predict::trajectory(&inp, predict::Flight::Pilot(&data.pilots[0]), None, now).unwrap();
        let pre_t =
            predict::trajectory(&inp, predict::Flight::Prefile(&data.prefiles[0]), None, now)
                .unwrap();
        assert_eq!(
            grd_t.allowance_sec(),
            660.0,
            "positive control: the samples were read"
        );
        assert_eq!(
            pre_t.allowance_sec(),
            660.0,
            "positive control: the samples were read"
        );
        let by = |cs: &str| flow.flights.iter().find(|f| f.callsign == cs).unwrap();

        let grd = by("GRD1");
        assert_eq!(grd.etd, Some(now + Duration::seconds(660)));
        assert_eq!(grd.eta, Some(grd_t.eta_at_field()), "the ETA doesn't move");

        let pre = by("PRE1");
        let filed = Utc.with_ymd_and_hms(2023, 11, 14, 23, 0, 0).unwrap();
        assert_eq!(pre.etd, Some(filed + Duration::seconds(660)));
        assert_eq!(pre.eta, Some(pre_t.eta_at_field()), "the ETA doesn't move");

        // Metered: the CFR backs out only the airborne ETE, so `etd_ms` moved with the field.
        for (f, t) in [(grd, &grd_t), (pre, &pre_t)] {
            let airborne_ms = (t.eta_at_field() - t.wheels_up()).num_milliseconds();
            assert_eq!(ms(f.sta) - ms(f.cfr), airborne_ms, "{}", f.callsign);
        }
    }

    /// AC #3 (cross-surface): the arrival ladder (`compute`) and the runway ETE
    /// (`runway::collect_arrivals`) must time the same aircraft identically — both route through
    /// `predict::trajectory` — and off the **resolved** filed route, not a straight line. The
    /// aircraft files a real NJ-coast routing (`RBV WHITE SIE`, bundled nav), so its along-route
    /// distance runs meaningfully longer than the great circle to the field; reverting
    /// the route resolution to `gc_dist` fails the distance assertion.
    #[test]
    fn ladder_and_runway_ete_agree_on_the_resolved_route() {
        let ap: AirportDb = HashMap::from([
            ("KJFK".to_string(), Airport::at(40.6413, -73.7781)),
            ("KDCA".to_string(), Airport::at(38.8521, -77.0377)),
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
        let flow = compute_with(
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
            &HashSet::new(),
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

    // ---- AADC bucketing (#242) ----

    #[test]
    fn carrier_of_extracts_the_leading_operator_prefix() {
        assert_eq!(carrier_of("AAL123"), Some("AAL".to_string()));
        assert_eq!(carrier_of("DAL1"), Some("DAL".to_string()));
        assert_eq!(carrier_of("N12345"), None); // bare tail number, 1-letter prefix
        assert_eq!(carrier_of("1234"), None); // no alphabetic prefix at all
    }

    #[test]
    fn bucket_aadc_assigns_flights_to_the_right_15min_bucket() {
        let flights = vec![
            ff("AAL1", "airborne", 10, Some("CAMRN")), // bucket 0 (0-15min)
            ff("DAL2", "ground", 50, Some("LENDY")),   // bucket 3 (45-60min)
        ];
        let buckets = bucket_aadc(&flights, t0(), 15);
        assert_eq!(buckets.len(), 16); // 4h / 15min
        assert_eq!(buckets[0].total, 1);
        assert_eq!(buckets[3].total, 1);
        assert_eq!(buckets[1].total, 0);
    }

    #[test]
    fn bucket_aadc_excludes_arrived_and_out_of_window_flights() {
        let flights = vec![
            FlowFlight {
                excluded: false,
                ..ff("UAL1", "arrived", 0, None)
            },
            // Already past (negative offset) — must not land in bucket 0.
            ff("SWA2", "airborne", -5, None),
            // Beyond the fixed 4-hour window.
            ff("JBU3", "proposed", 300, None),
            // Excluded from metering — should still be excluded from AADC demand too.
            FlowFlight {
                excluded: true,
                ..ff("FDX4", "ground", 30, None)
            },
        ];
        let buckets = bucket_aadc(&flights, t0(), 15);
        let total: i64 = buckets.iter().map(|b| b.total).sum();
        assert_eq!(total, 0, "no flight here should count toward demand");
    }

    #[test]
    fn bucket_aadc_breaks_down_every_dimension() {
        let flights = vec![FlowFlight {
            category: Some("H".into()),
            ..ff("AAL123", "airborne", 5, Some("CAMRN"))
        }];
        let buckets = bucket_aadc(&flights, t0(), 15);
        let b = &buckets[0];
        assert_eq!(b.by_status.get("airborne"), Some(&1));
        assert_eq!(b.by_category.get("H"), Some(&1));
        assert_eq!(b.by_carrier.get("AAL"), Some(&1));
        assert_eq!(b.by_afix.get("CAMRN"), Some(&1));
    }

    #[test]
    fn bucket_aadc_caps_carrier_breakdown_to_top_n_plus_other() {
        // 9 distinct single-flight carriers (AAA..III) plus a 10th (JJJ) with 2 flights. JJJ
        // outranks everyone and takes 1 of the 8 top-carrier slots; among the 9 tied-at-1
        // carriers only the 7 alphabetically-first fit the remaining slots — HHH and III must
        // fold into OTHER (2 flights).
        let prefixes = [
            "AAA", "BBB", "CCC", "DDD", "EEE", "FFF", "GGG", "HHH", "III",
        ];
        let mut flights: Vec<FlowFlight> = prefixes
            .iter()
            .map(|p| ff(&format!("{p}1"), "airborne", 5, None))
            .collect();
        flights.push(ff("JJJ1", "airborne", 5, None));
        flights.push(ff("JJJ2", "airborne", 5, None));

        let buckets = bucket_aadc(&flights, t0(), 15);
        let by_carrier = &buckets[0].by_carrier;
        assert_eq!(
            by_carrier.get("JJJ"),
            Some(&2),
            "the busiest carrier must survive the cap"
        );
        assert_eq!(
            by_carrier.len(),
            AADC_TOP_CARRIERS + 1,
            "top N carriers plus one OTHER bucket"
        );
        assert_eq!(
            by_carrier.get("HHH"),
            None,
            "a lowest-ranked carrier must be dropped"
        );
        assert_eq!(
            by_carrier.get("III"),
            None,
            "a lowest-ranked carrier must be dropped"
        );
        assert_eq!(by_carrier.get(AADC_OTHER), Some(&2));
    }

    fn gate(id: &str, lat: f64, lon: f64) -> AirportGateBody {
        AirportGateBody {
            id: id.to_string(),
            icao: "KAAA".to_string(),
            name: id.to_string(),
            lat,
            lon,
            source: "manual".to_string(),
            kind: None,
            // `nearest_gate` reads only id/lat/lon/name, so the X-Plane detail is None here for the
            // same reason `kind` is: this helper builds a hand-entered stand (#541).
            heading: None,
            size_code: None,
            operation_type: None,
            aircraft_classes: None,
            airline_codes: None,
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
            None,
        );
        assert_eq!(allowance, predict::GROUND_TAXI_SEC + 300.0);
    }

    /// No position (a prefile) skips gate/runway
    /// matching entirely rather than guessing — still resolves via the airport/default tier.
    #[test]
    fn resolve_ground_allowance_sec_skips_gate_and_runway_matching_without_a_position() {
        let gates = HashMap::from([("KAAA".to_string(), vec![gate("A1", 40.0, -74.0)])]);
        let runways = RunwayDb::default();
        let samples = HashMap::new();
        let allowance = resolve_ground_allowance_sec(
            &gates,
            &runways,
            &samples,
            "KAAA",
            Some("B738"),
            None,
            None,
        );
        assert_eq!(allowance, predict::GROUND_TAXI_SEC + 300.0);
    }

    fn taxi_sample(
        gate: &str,
        aircraft: &str,
        runway: &str,
        taxi_sec: i32,
    ) -> taxi_estimate::TaxiSample {
        taxi_estimate::TaxiSample {
            gate_id: Some(gate.into()),
            aircraft: Some(aircraft.into()),
            runway: Some(runway.into()),
            pushback_sec: Some(50),
            startup_sec: Some(40),
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
            None,
        );
        // Airport-wide blend (median of the combined 10 samples), not the runway-specific 600s.
        assert_eq!(stationary, 360.0 + 50.0 + 40.0);

        let rolling = resolve_ground_allowance_sec(
            &gates,
            &runways,
            &samples,
            "KJFK",
            Some("B738"),
            Some((40.0, -74.0, end.hdg as i64, TAXI_ROLL_GS_KT + 1)),
            None,
        );
        // Once actually moving, the same heading legitimately resolves the gate/type/runway tier.
        assert_eq!(rolling, 600.0 + 50.0 + 40.0);
    }

    /// #513 AC2: a **parked** departure with an assigned runway reaches the gate/type/runway tier, where
    /// the same aircraft without one falls to the airport-wide blend.
    ///
    /// This is the whole point of the epic. A parked aircraft's heading is refused on purpose, so before
    /// #513 every parked and prefiled departure took the airport/default tier — and this allowance
    /// sets the earliest wheels-up behind the ETA and the earliest EDCT.
    #[test]
    fn an_assigned_runway_reaches_the_runway_tier_while_parked() {
        let runways = RunwayDb::load();
        let gates = HashMap::from([("KJFK".to_string(), vec![gate("A1", 40.0, -74.0)])]);
        // Five runway-specific samples at 600s, five airport-wide at 120s: the tiers differ, so which
        // one was matched is visible in the number.
        let mut rows: Vec<taxi_estimate::TaxiSample> = (0..5)
            .map(|_| taxi_sample("A1", "B738", "04L", 600))
            .collect();
        rows.extend((0..5).map(|_| taxi_sample("B9", "A320", "22R", 120)));
        let samples = HashMap::from([("KJFK".to_string(), rows)]);
        // Stationary on stand A1 — no usable heading.
        let parked = Some((40.0, -74.0, 0, 0));

        let without = resolve_ground_allowance_sec(
            &gates,
            &runways,
            &samples,
            "KJFK",
            Some("B738"),
            parked,
            None,
        );
        let with = resolve_ground_allowance_sec(
            &gates,
            &runways,
            &samples,
            "KJFK",
            Some("B738"),
            parked,
            Some("04L"),
        );

        assert_ne!(
            with, without,
            "an assigned runway must change the estimate for a parked departure, or #513 did nothing"
        );
        assert_eq!(
            with,
            600.0 + 50.0 + 40.0,
            "the assignment reaches the gate/type/runway tier (600s), not the airport blend"
        );
    }

    /// #513 AC1: an assignment **replaces** the heading guess rather than sitting beside it. The aircraft
    /// is rolling and pointed at 04L, so the guess would pick 04L — the assignment says 22R, and 22R is
    /// what the estimate must use.
    #[test]
    fn an_assigned_runway_overrides_the_heading_guess_while_rolling() {
        let runways = RunwayDb::load();
        let end = runways
            .ends_for("KJFK")
            .into_iter()
            .find(|e| e.id == "04L")
            .expect("KJFK 04L is in the bundled runway data");
        let gates = HashMap::from([("KJFK".to_string(), vec![gate("A1", 40.0, -74.0)])]);
        // 04L samples are slow, 22R samples are fast: the runway actually used is legible in the total.
        let mut rows: Vec<taxi_estimate::TaxiSample> = (0..5)
            .map(|_| taxi_sample("A1", "B738", "04L", 900))
            .collect();
        rows.extend((0..5).map(|_| taxi_sample("A1", "B738", "22R", 300)));
        let samples = HashMap::from([("KJFK".to_string(), rows)]);
        // Rolling, pointed down 04L — exactly the case where the guess is trusted.
        let rolling = Some((40.0, -74.0, end.hdg as i64, TAXI_ROLL_GS_KT + 1));

        let breakdown = resolve_ground_allowance(
            &gates,
            &runways,
            &samples,
            "KJFK",
            Some("B738"),
            rolling,
            Some("22R"),
        );

        assert_eq!(
            breakdown.runway.as_deref(),
            Some("22R"),
            "the decision must win over the heading inference, not be averaged with it"
        );
    }

    /// #513 AC4: with no assignment the heading guess still applies exactly as before.
    #[test]
    fn no_assignment_leaves_the_heading_guess_in_charge() {
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
            None,
        );

        assert_eq!(
            breakdown.runway.as_deref(),
            Some("04L"),
            "the guess still works"
        );
    }

    /// An empty or whitespace assignment is treated as no assignment, not as a runway named "".
    #[test]
    fn a_blank_assignment_is_no_assignment() {
        let runways = RunwayDb::load();
        let gates = HashMap::from([("KJFK".to_string(), vec![gate("A1", 40.0, -74.0)])]);
        let samples = HashMap::new();

        for blank in ["", "   "] {
            let breakdown = resolve_ground_allowance(
                &gates,
                &runways,
                &samples,
                "KJFK",
                Some("B738"),
                Some((40.0, -74.0, 0, 0)),
                Some(blank),
            );
            assert_eq!(breakdown.runway, None, "{blank:?} must not become a runway");
        }
    }

    /// An assignment is upper-cased, so a lower-case value from a rule still matches the stored samples.
    #[test]
    fn an_assignment_is_matched_case_insensitively() {
        let runways = RunwayDb::load();
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
            Some((40.0, -74.0, 0, 0)),
            Some("04l"),
        );

        assert_eq!(breakdown.runway.as_deref(), Some("04L"));
    }

    /// #513 AC5: the number a controller sees actually moves. The allowance sets the earliest
    /// wheels-up, and with it the ETA and the earliest EDCT — so two different assigned runways, whose
    /// learned samples differ, must yield different totals. Without this the sub-issue cannot be shown
    /// to have changed anything a controller reads.
    #[test]
    fn a_different_assigned_runway_moves_the_modeled_transit() {
        let runways = RunwayDb::load();
        let gates = HashMap::from([("KJFK".to_string(), vec![gate("A1", 40.0, -74.0)])]);
        let mut rows: Vec<taxi_estimate::TaxiSample> = (0..5)
            .map(|_| taxi_sample("A1", "B738", "04L", 900))
            .collect();
        rows.extend((0..5).map(|_| taxi_sample("A1", "B738", "22R", 300)));
        let samples = HashMap::from([("KJFK".to_string(), rows)]);
        let parked = Some((40.0, -74.0, 0, 0));

        let slow = resolve_ground_allowance_sec(
            &gates,
            &runways,
            &samples,
            "KJFK",
            Some("B738"),
            parked,
            Some("04L"),
        );
        let quick = resolve_ground_allowance_sec(
            &gates,
            &runways,
            &samples,
            "KJFK",
            Some("B738"),
            parked,
            Some("22R"),
        );

        assert!(
            slow > quick,
            "the 900s runway must model a longer transit than the 300s one ({slow} vs {quick})"
        );
        // And the difference is the samples' difference, not a rounding artefact — a 600s swing in the
        // earliest wheels-up for this flight.
        assert_eq!(slow - quick, 600.0);
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
            None,
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
        assert_eq!(breakdown.startup.value_sec, 40.0);
        assert_eq!(breakdown.total_sec(), 690.0);

        // A thin-data airport falls to the default tier with no matched key at all.
        let default_breakdown = resolve_ground_allowance(
            &HashMap::new(),
            &RunwayDb::default(),
            &HashMap::new(),
            "KAAA",
            Some("B738"),
            Some((40.0, -74.0, 270, 20)),
            None,
        );
        assert_eq!(default_breakdown.gate_id, None);
        assert_eq!(default_breakdown.runway, None);
        assert_eq!(
            default_breakdown.taxi.tier,
            taxi_estimate::EstimateTier::Default
        );
    }

    /// Airport-flow MIT is converted at the trailing flight's predicted groundspeed 40 nm short of
    /// the field (`MIT_CROSSING_NM`), read off its trajectory: below cruise for a jet descending
    /// into the field, the speed predicted where it is now once it is inside 40 nm.
    #[test]
    fn compute_converts_mit_at_the_speed_forty_miles_out() {
        let ap = AirportDb::from([
            ("KJFK".to_string(), Airport::at(40.64, -73.78)),
            ("KDCA".to_string(), Airport::at(38.85, -77.04)),
        ]);
        let plan = FlightPlan {
            departure: "KJFK".into(),
            arrival: "KDCA".into(),
            altitude: "35000".into(),
            cruise_tas: "440".into(),
            ..Default::default()
        };
        let pilot = |cs: &str, pos: [f64; 2], alt, gs| Pilot {
            callsign: cs.into(),
            latitude: pos[0],
            longitude: pos[1],
            altitude: alt,
            groundspeed: gs,
            heading: 230,
            flight_plan: Some(plan.clone()),
            ..Default::default()
        };
        let data = VatsimData {
            pilots: vec![
                pilot("FAR1", [40.4, -74.2], 35_000, 440),
                pilot("NEAR1", [38.95, -76.75], 3_000, 180),
            ],
            ..Default::default()
        };
        let (nav, winds, profiles) = (
            NavData::default(),
            Winds::default(),
            trajectory::ProfileTable::default(),
        );
        let no_samples = HashMap::new();
        let inp = inputs_with(&ap, &nav, &winds, &profiles, &no_samples);
        let flow = compute(
            "KDCA",
            None,
            &data,
            &inp,
            &HashMap::new(),
            &HashSet::new(),
            t0(),
        );
        for (i, cs) in ["FAR1", "NEAR1"].into_iter().enumerate() {
            let traj =
                predict::trajectory(&inp, predict::Flight::Pilot(&data.pilots[i]), None, t0())
                    .unwrap();
            let row = flow.flights.iter().find(|f| f.callsign == cs).unwrap();
            // The literal 40 nm the MIT rule is defined at, not the constant (#813).
            assert_eq!(row.mit_cross_kt, traj.gs_short_of_field(40.0), "{cs}");
        }
        // Positive control: FAR1's speed differs at 30 and 60 nm, so a moved point shows.
        let far_traj =
            predict::trajectory(&inp, predict::Flight::Pilot(&data.pilots[0]), None, t0()).unwrap();
        for nm in [30.0, 60.0] {
            assert_ne!(
                far_traj.gs_short_of_field(nm),
                far_traj.gs_short_of_field(40.0),
                "{nm} nm"
            );
        }
        let far = flow.flights.iter().find(|f| f.callsign == "FAR1").unwrap();
        assert!(
            far.mit_cross_kt < 400.0,
            "a descent speed, not the 440 kt cruise: {}",
            far.mit_cross_kt
        );
        // NEAR1 is ~15 nm out: the speed predicted where it is now.
        let near_traj =
            predict::trajectory(&inp, predict::Flight::Pilot(&data.pilots[1]), None, t0()).unwrap();
        assert!(
            near_traj.route_nm() < MIT_CROSSING_NM,
            "positive control: inside 40 nm"
        );
        let near = flow.flights.iter().find(|f| f.callsign == "NEAR1").unwrap();
        assert_eq!(near.mit_cross_kt, near_traj.gs_at(0.0), "inside 40 nm");
        assert!(far.mit_cross_kt > near.mit_cross_kt);
    }

    /// Only a pilot on the ground within 5 nm of the field has arrived: one on a 3 nm final at 140 kt
    /// is still an airborne inbound with a metered slot.
    #[test]
    fn a_flight_on_short_final_has_not_arrived() {
        let ap = AirportDb::from([
            ("KJFK".to_string(), Airport::at(40.64, -73.78)),
            ("KDCA".to_string(), Airport::at(38.85, -77.04)),
        ]);
        let plan = FlightPlan {
            departure: "KJFK".into(),
            arrival: "KDCA".into(),
            altitude: "35000".into(),
            cruise_tas: "440".into(),
            ..Default::default()
        };
        let pilot = |cs: &str, gs, alt| Pilot {
            callsign: cs.into(),
            latitude: 38.90,
            longitude: -77.04,
            altitude: alt,
            groundspeed: gs,
            heading: 180,
            flight_plan: Some(plan.clone()),
            ..Default::default()
        };
        let data = VatsimData {
            pilots: vec![pilot("FINAL", 140, 1_000), pilot("PARKED", 0, 0)],
            ..Default::default()
        };
        let (nav, winds, profiles) = (
            NavData::default(),
            Winds::default(),
            trajectory::ProfileTable::default(),
        );
        let no_samples = HashMap::new();
        let inp = inputs_with(&ap, &nav, &winds, &profiles, &no_samples);
        let pg = base_program_aar(60);
        let flow = compute(
            "KDCA",
            Some(&pg),
            &data,
            &inp,
            &HashMap::new(),
            &HashSet::new(),
            t0(),
        );
        let row = |cs: &str| flow.flights.iter().find(|f| f.callsign == cs).unwrap();
        assert_eq!(row("FINAL").status, "airborne");
        assert!(row("FINAL").sta.is_some(), "metered");
        assert_eq!(row("PARKED").status, "arrived", "positive control");
    }

    /// Airport flow labels a pilot airborne from 50 kt (`predict::is_airborne`), gives an airborne
    /// row no wheels-up or CFR under a metered program, and drops a prefile that shares a connected
    /// pilot's callsign, so demand counts the aircraft once.
    #[test]
    fn compute_labels_times_and_dedupes_each_inbound_once() {
        let ap = AirportDb::from([
            ("KJFK".to_string(), Airport::at(40.64, -73.78)),
            ("KDCA".to_string(), Airport::at(38.85, -77.04)),
        ]);
        let plan = FlightPlan {
            departure: "KJFK".into(),
            arrival: "KDCA".into(),
            altitude: "35000".into(),
            cruise_tas: "440".into(),
            deptime: "0000".into(),
            ..Default::default()
        };
        let pilot = |cs: &str, gs, alt| Pilot {
            callsign: cs.into(),
            latitude: 40.64,
            longitude: -73.78,
            altitude: alt,
            groundspeed: gs,
            heading: 230,
            flight_plan: Some(plan.clone()),
            ..Default::default()
        };
        let data = VatsimData {
            pilots: vec![
                pilot("ROLL", 55, 0),
                pilot("LOW", 70, 200),
                pilot("TAXI", 45, 0),
            ],
            prefiles: vec![Prefile {
                callsign: "TAXI".into(),
                flight_plan: Some(plan.clone()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let (nav, winds, profiles) = (
            NavData::default(),
            Winds::default(),
            trajectory::ProfileTable::default(),
        );
        let no_samples = HashMap::new();
        let inp = inputs_with(&ap, &nav, &winds, &profiles, &no_samples);
        let pg = base_program_aar(60);
        let flow = compute(
            "KDCA",
            Some(&pg),
            &data,
            &inp,
            &HashMap::new(),
            &HashSet::new(),
            t0(),
        );
        let row = |cs: &str| {
            let rows: Vec<_> = flow.flights.iter().filter(|f| f.callsign == cs).collect();
            assert_eq!(rows.len(), 1, "{cs} listed once");
            rows[0]
        };
        for cs in ["ROLL", "LOW"] {
            let f = row(cs);
            assert_eq!(f.status, "airborne", "{cs}");
            assert!(f.etd.is_none() && f.cfr.is_none(), "{cs} has no wheels-up");
            assert!(f.sta.is_some(), "positive control: {cs} is metered");
        }
        let taxi = row("TAXI");
        assert_eq!(taxi.status, "ground");
        assert!(
            taxi.etd.is_some(),
            "positive control: a ground row has a wheels-up"
        );
        assert_eq!((flow.airborne, flow.ground, flow.proposed), (2, 1, 0));
    }

    /// #335: the nominal 300 nm leg, taken for a prefile whose departure is not in the `AirportDb`,
    /// descends to the arrival field's elevation; reading sea level instead left the suite green. The
    /// mutation this pins is neutering `predict::profile_from_here`'s
    /// `field_elevation_ft(airports, arr_icao)` lookup to `0.0`.
    #[test]
    fn the_nominal_ground_leg_descends_to_the_arrival_field() {
        let estimate = |elevation_ft: f64| {
            let ap = AirportDb::from([(
                "KDEN".to_string(),
                Airport {
                    elevation_ft,
                    ..Airport::at(39.86, -104.67)
                },
            )]);
            let pf = Prefile {
                callsign: "PRE1".into(),
                flight_plan: Some(FlightPlan {
                    departure: "KJFK".into(), // absent from `ap`
                    arrival: "KDEN".into(),
                    altitude: "35000".into(),
                    cruise_tas: "440".into(),
                    ..Default::default()
                }),
                ..Default::default()
            };
            let (nav, winds, profiles) = (
                NavData::default(),
                Winds::default(),
                trajectory::ProfileTable::default(),
            );
            let no_samples = HashMap::new();
            let inp = inputs_with(&ap, &nav, &winds, &profiles, &no_samples);
            let traj =
                predict::trajectory(&inp, predict::Flight::Prefile(&pf), None, t0()).unwrap();
            (
                traj.route_nm(),
                (traj.eta_at_field() - traj.wheels_up()).num_milliseconds(),
            )
        };

        let (sea_nm, sea_ms) = estimate(0.0);
        let (high_nm, high_ms) = estimate(5431.0);

        // The nominal 300 nm leg, not a resolved route.
        assert_eq!(sea_nm, 300.0);
        assert_eq!(high_nm, 300.0);
        assert!(
            high_ms < sea_ms,
            "a 5431 ft field shortens the descent, so the flight time must come in below a \
             sea-level one — got {high_ms} ms vs {sea_ms} ms"
        );
    }

    // ---- #836: airport-flow demand reads the per-segment wind (AC5) ---------------------------
    //
    // `Winds::test_shear()`: 80 kt from 225° (on the nose of a south-west KJFK→KDCA course) at and
    // above 30,000 ft east of 75.5°W, calm elsewhere.

    fn jfk_dca() -> AirportDb {
        HashMap::from([
            ("KJFK".to_string(), Airport::at(40.6413, -73.7781)),
            ("KDCA".to_string(), Airport::at(38.8521, -77.0377)),
        ])
    }

    fn ladder(data: &VatsimData, winds: &Winds, cs: &str) -> DateTime<Utc> {
        compute_with(
            "KDCA",
            None,
            data,
            &jfk_dca(),
            &NavData::load(),
            winds,
            &trajectory::ProfileTable::default(),
            &HashMap::new(),
            &HashMap::new(),
            &RunwayDb::default(),
            &HashMap::new(),
            &HashSet::new(),
            t0(),
        )
        .flights
        .into_iter()
        .find(|f| f.callsign == cs)
        .and_then(|f| f.eta)
        .expect("ladder ETA")
    }

    /// The coastal B738 of `ladder_and_runway_ete_agree_on_the_resolved_route`.
    fn coastal() -> VatsimData {
        let mut p = pilot(
            "AAL1",
            40.2,
            -74.0,
            24_000,
            400,
            fp("KJFK", "KDCA", "RBV WHITE SIE"),
        );
        p.heading = 220;
        VatsimData {
            pilots: vec![p],
            ..Default::default()
        }
    }

    // AC5: the airborne arrival site.
    #[test]
    fn compute_times_an_airborne_arrival_in_the_route_wind() {
        let data = coastal();
        let shear = Winds::test_shear();
        let windy = ladder(&data, &shear, "AAL1");
        let still = ladder(&data, &Winds::default(), "AAL1");
        assert!(
            (windy - still).num_seconds() > 30,
            "{} s",
            (windy - still).num_seconds()
        );
        let (ap, nav, profiles, no_samples) = (
            jfk_dca(),
            NavData::load(),
            trajectory::ProfileTable::default(),
            HashMap::new(),
        );
        let inp = inputs_with(&ap, &nav, &shear, &profiles, &no_samples);
        let direct =
            predict::trajectory(&inp, predict::Flight::Pilot(&data.pilots[0]), None, t0()).unwrap();
        assert!(direct.airborne(), "positive control: the airborne branch");
        assert_eq!(windy, direct.eta_at_field());
    }

    // AC5: the ground / prefile site (`predict::trajectory` for a parked pilot).
    #[test]
    fn compute_times_a_proposed_departure_in_the_route_wind() {
        let parked = pilot(
            "DAL2",
            40.6413,
            -73.7781,
            0,
            0,
            fp("KJFK", "KDCA", "RBV WHITE SIE"),
        );
        let data = VatsimData {
            pilots: vec![parked],
            prefiles: vec![Prefile {
                callsign: "UAL3".into(),
                flight_plan: Some(fp("KJFK", "KDCA", "RBV WHITE SIE")),
                ..Default::default()
            }],
            ..Default::default()
        };
        let shear = Winds::test_shear();
        for cs in ["DAL2", "UAL3"] {
            let windy = ladder(&data, &shear, cs);
            let still = ladder(&data, &Winds::default(), cs);
            assert!(
                (windy - still).num_seconds() > 20,
                "{cs}: {} s",
                (windy - still).num_seconds()
            );
        }
        // Each ETA is the shared trajectory's, the pilot from where it is parked and the prefile from
        // its departure field.
        let (ap, nav, profiles, no_samples) = (
            jfk_dca(),
            NavData::load(),
            trajectory::ProfileTable::default(),
            HashMap::new(),
        );
        let inp = inputs_with(&ap, &nav, &shear, &profiles, &no_samples);
        let pilot = predict::trajectory(&inp, predict::Flight::Pilot(&data.pilots[0]), None, t0());
        let pre = predict::trajectory(
            &inp,
            predict::Flight::Prefile(&data.prefiles[0]),
            None,
            t0(),
        );
        assert_eq!(ladder(&data, &shear, "DAL2"), pilot.unwrap().eta_at_field());
        assert_eq!(ladder(&data, &shear, "UAL3"), pre.unwrap().eta_at_field());
    }

    // By design: with no departure coordinates there is no route to read wind along.
    #[test]
    fn the_nominal_300_nm_leg_flies_still_air() {
        let pf = Prefile {
            callsign: "PRE1".into(),
            flight_plan: Some(fp("KZZZ", "KDCA", "")),
            ..Default::default()
        };
        let (ap, nav, profiles, no_samples) = (
            jfk_dca(),
            NavData::default(),
            trajectory::ProfileTable::default(),
            HashMap::new(),
        );
        let shear = Winds::test_shear();
        let inp = inputs_with(&ap, &nav, &shear, &profiles, &no_samples);
        let traj = predict::trajectory(&inp, predict::Flight::Prefile(&pf), None, t0()).unwrap();
        assert_eq!(traj.route_nm(), 300.0, "positive control: the nominal leg");
        assert!(traj.path().is_empty());
        // The airborne minutes the model gives this plan over 300 nm in still air, a literal so a
        // wind applied on this path moves the result.
        let airborne =
            (traj.eta_at_field() - traj.wheels_up()).num_milliseconds() as f64 / 60_000.0;
        // The ETA is whole seconds, so allow one.
        assert!((airborne - 47.382).abs() < 1.0 / 60.0, "{airborne}");
    }

    // AC5 cross-surface: ladder and runway ETE agree when the wind is not uniform.
    #[test]
    fn ladder_and_runway_ete_agree_under_a_sheared_wind() {
        let data = coastal();
        let shear = Winds::test_shear();
        let eta = ladder(&data, &shear, "AAL1");
        let ete = crate::feed::runway::collect_arrivals(
            "KDCA",
            &data,
            &jfk_dca(),
            &NavData::load(),
            &shear,
            &trajectory::ProfileTable::default(),
            t0(),
            600,
        )
        .into_iter()
        .find(|a| a.cs == "AAL1")
        .expect("runway ETE")
        .eta_ms;
        assert_eq!(eta.timestamp_millis(), ete);
        let still = ladder(&data, &Winds::default(), "AAL1");
        assert!((eta - still).num_seconds() > 30);
    }

    /// #813: a connected ground pilot is timed from where it sits, so a departure field missing from
    /// the `AirportDb` still gets its great circle to the arrival (padded, the route being direct),
    /// and its wheels-up is one allowance after now (#815).
    #[test]
    fn a_ground_pilot_at_an_unknown_field_is_timed_from_where_it_sits() {
        let now = t0();
        // KXXX has no coordinates in `airports()`. Five samples there and a parked pilot: the
        // airport tier, 50 + 40 + 570 s.
        let samples = HashMap::from([(
            "KXXX".to_string(),
            (0..5)
                .map(|_| taxi_sample("A1", "B738", "04L", 570))
                .collect::<Vec<_>>(),
        )]);
        let plan = fp("KXXX", "KJFK", "DCT");
        let data = VatsimData {
            pilots: vec![pilot("GRD1", 42.36, -71.0, 0, 0, plan)],
            ..Default::default()
        };
        let flow = compute_with(
            "KJFK",
            None,
            &data,
            &airports(),
            &NavData::default(),
            &Winds::default(),
            &trajectory::ProfileTable::default(),
            &HashMap::new(),
            &HashMap::new(),
            &RunwayDb::default(),
            &samples,
            &HashSet::new(),
            now,
        );
        let gc = gc_dist(42.36, -71.0, 40.6413, -73.7781);
        let d = flow.flights[0].distance_nm.unwrap();
        assert!(
            (d - gc * predict::GROUND_ROUTE_FACTOR).abs() < 1e-6,
            "{d} nm against a {gc} nm great circle"
        );
        assert_eq!(flow.flights[0].etd, Some(now + Duration::seconds(660)));
    }
}
