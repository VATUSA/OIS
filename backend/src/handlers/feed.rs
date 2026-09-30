//! Live-feed handlers: feed health, per-airport arrival flow (metered against a program),
//! the departure-field CFR view, and issuing/releasing CFRs.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use utoipa::ToSchema;

use crate::{
    auth::{
        context::CurrentUser,
        permissions::{TmuCfrAssign, TmuProgramRead},
        require_permission::RequirePermission,
    },
    errors::ApiError,
    feed,
    feed::facilities,
    feed::flow::{self, ProgramInputs},
    feed::taxi,
    models::{DepartureFlight, DeparturesResponse, IssueCfrRequest, IssuedCfrBody},
    repos::airport_configs as config_repo,
    repos::{flow as flow_repo, tmu as tmu_repo},
    state::AppState,
};

#[derive(Debug, Serialize, ToSchema)]
pub struct FeedStatusBody {
    /// True when the last datafeed fetch succeeded.
    pub healthy: bool,
    /// When OIS last ingested the feed.
    pub last_updated: Option<DateTime<Utc>>,
    /// The feed's own `update_timestamp` from VATSIM.
    pub source_timestamp: Option<String>,
    pub last_error: Option<String>,
    pub pilots: usize,
    pub prefiles: usize,
    pub airports_loaded: usize,
}

#[utoipa::path(
    get,
    path = "/api/v1/feed/status",
    tag = "feed",
    responses((status = 200, body = FeedStatusBody), (status = 401))
)]
pub async fn feed_status(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuProgramRead>,
) -> Json<FeedStatusBody> {
    let guard = state.feed.read().await;
    let s = &guard.status;
    Json(FeedStatusBody {
        healthy: s.healthy,
        last_updated: s.last_ok,
        source_timestamp: s.source_timestamp.clone(),
        last_error: s.last_error.clone(),
        pilots: s.pilots,
        prefiles: s.prefiles,
        airports_loaded: s.airports_loaded,
    })
}

/// Build a program's metering inputs from its stored row.
pub(crate) async fn program_inputs(
    pool: &PgPool,
    icao: &str,
) -> Result<Option<ProgramInputs>, ApiError> {
    Ok(tmu_repo::get_program(pool, icao)
        .await?
        .map(|p| ProgramInputs {
            aar: p.aar,
            trail: p.trail,
            mit: p.mit,
            gates: p
                .gates
                .0
                .iter()
                .map(|g| flow::GateSpacing {
                    name: g.name.clone(),
                    trail: g.trail,
                    mit: g.mit,
                })
                .collect(),
            exclude_wake: p.exclude_wake,
            exclude_types: p.exclude_types,
            jets_only: p.jets_only,
        }))
}

/// Compute the metered flow for one arrival airport against a given snapshot at `now` (loads the
/// program + issued CFRs from the DB). Shared by the live handler and the historical replay — the
/// only difference is which snapshot and instant are passed in.
pub(crate) async fn flow_from_data(
    state: &AppState,
    pool: &PgPool,
    icao: &str,
    snap: Arc<crate::feed::Snapshot>,
    winds: Arc<crate::feed::winds::Winds>,
    now: DateTime<Utc>,
) -> Result<flow::Flow, ApiError> {
    let program = program_inputs(pool, icao).await?;
    let issued = tmu_repo::issued_cfr_map(pool, icao).await?;
    let airports = state.feed.read().await.airports.clone();
    let nav = state.nav.load_full();
    let profiles = state.aircraft_profiles.load_full();
    let gates = state.gates.load_full();
    let runways = state.runways.clone();
    let taxi_estimate_samples = state.taxi_estimate_samples.load_full();
    let manual_exclusions =
        crate::handlers::flow::all_excluded_callsigns(state.flight_exclusions.load().as_ref());
    let icao = icao.to_owned();
    // `compute` resolves every arrival's filed route — pure CPU, no `.await`. Push it onto the
    // blocking pool (matching the FCA `metered_flights` handler) so a burst of polling clients
    // can't stall the async runtime.
    tokio::task::spawn_blocking(move || {
        flow::compute(
            &icao,
            program.as_ref(),
            &snap.data,
            airports.as_ref(),
            nav.as_ref(),
            winds.as_ref(),
            profiles.as_ref(),
            &issued,
            gates.as_ref(),
            runways.as_ref(),
            taxi_estimate_samples.as_ref(),
            &manual_exclusions,
            now,
        )
    })
    .await
    .map_err(|_| ApiError::Internal)
}

/// Compute the live, metered flow for one arrival airport (loads program + issued CFRs).
pub(crate) async fn flow_for(
    state: &AppState,
    pool: &PgPool,
    icao: &str,
) -> Result<flow::Flow, ApiError> {
    // Clone the snapshot + airport handles and drop the feed lock before metering.
    let snapshot = state.feed.read().await.snapshot.clone();
    match snapshot {
        Some(snap) => {
            let winds = state.winds.load_full();
            flow_from_data(state, pool, icao, snap, winds, Utc::now()).await
        }
        None => Ok(flow::Flow {
            icao: icao.to_string(),
            aar: program_inputs(pool, icao).await?.map(|p| p.aar),
            inbound: 0,
            airborne: 0,
            ground: 0,
            proposed: 0,
            demand_60min: 0,
            over_capacity: None,
            flights: Vec::new(),
        }),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/tmu/flow/{icao}",
    tag = "tmu",
    params(("icao" = String, Path, description = "Arrival airport ICAO")),
    responses((status = 200, body = crate::feed::flow::Flow), (status = 401), (status = 503))
)]
pub async fn airport_flow(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuProgramRead>,
    Path(icao): Path<String>,
) -> Result<Json<flow::Flow>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let icao = icao.trim().to_ascii_uppercase();
    Ok(Json(flow_for(&state, pool, &icao).await?))
}

#[derive(Deserialize)]
pub struct AadcQuery {
    /// Bucket width in minutes: 15, 30, or 60. Defaults to 15.
    pub bucket_min: Option<i32>,
}

#[utoipa::path(
    get,
    path = "/api/v1/tmu/flow/{icao}/aadc",
    tag = "tmu",
    params(
        ("icao" = String, Path, description = "Arrival airport ICAO"),
        ("bucket_min" = Option<i32>, Query, description = "Bucket width in minutes: 15, 30, or 60"),
    ),
    responses(
        (status = 200, body = crate::feed::flow::AadcResponse),
        (status = 400),
        (status = 401),
        (status = 503),
    )
)]
pub async fn airport_aadc(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuProgramRead>,
    Path(icao): Path<String>,
    Query(q): Query<AadcQuery>,
) -> Result<Json<flow::AadcResponse>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let icao = icao.trim().to_ascii_uppercase();
    let bucket_min = q.bucket_min.unwrap_or(15);
    if !matches!(bucket_min, 15 | 30 | 60) {
        return Err(ApiError::BadRequest);
    }

    let live = flow_for(&state, pool, &icao).await?;
    let now = Utc::now();

    let configs = config_repo::list_by_icao(pool, &icao).await?;
    let airports = state.feed.read().await.airports.clone();
    let wind_dir = feed::forecast::wind_at(&airports, &icao, now)
        .await
        .and_then(|h| h.dir);
    let favored = config_repo::favored_config(&configs, wind_dir);
    let (aar, adr, config_id) = favored
        .map(|c| (c.aar, c.adr, Some(c.id.clone())))
        .unwrap_or((0, 0, None));

    let buckets = flow::bucket_aadc(&live.flights, now, bucket_min);

    Ok(Json(flow::AadcResponse {
        icao,
        bucket_min,
        aar,
        adr,
        config_id,
        buckets,
        generated_at: now,
    }))
}

/// The capacity an airport's demand is ranked against, with where it came from.
///
/// A metering program's AAR wins: it is the rate that actually meters traffic
/// (`feed::flow::compute`'s `apply_metering`), so ranking against anything else would contradict
/// the sequence controllers are working. Otherwise fall back to the airport's runway config.
///
/// The config branch passes `None` for the wind, which `favored_config` documents as always falling
/// through to the calm-default (else the first config). Resolving the *wind-favored* config costs
/// one outbound Open-Meteo request per airport (`feed::forecast::wind_at`, 15s timeout behind a
/// process-global mutex), which a national aggregate cannot afford. An airport under metering is
/// unaffected, since its program AAR wins here anyway.
///
/// `None` when the airport has neither a program nor a config — it is left out of the ranking
/// rather than ranked against a capacity nobody set.
fn effective_aar(
    program: Option<&ProgramInputs>,
    configs: &[crate::models::AirportConfigBody],
) -> Option<(i32, &'static str)> {
    if let Some(p) = program {
        return Some((p.aar, "program"));
    }
    config_repo::favored_config(configs, None).map(|c| (c.aar, "config"))
}

#[utoipa::path(
    get,
    path = "/api/v1/tmu/demand",
    tag = "tmu",
    responses(
        (status = 200, body = Vec<crate::models::AirportDemandBody>),
        (status = 401),
        (status = 503),
    )
)]
/// Current arrival demand vs capacity for every airport at once, ranked by exceedance
/// (VATUSA/OIS#475).
///
/// Shaped like `flow::fca_counts`, not like the per-airport handlers above: a fixed three queries up
/// front, one brief feed read, then a *single* blocking task for the whole loop. Fanning
/// `flow_for` out per airport (as `public::get_board` does for the handful of metered fields) would
/// be two queries and a blocking task each.
///
/// Gated coarsely on `tmu.program.read` at any scope, like the sibling feed reads — the ranking is
/// derived from public VATSIM traffic, so a facility-scoped caller gets it too. "National" is a
/// menu-curation decision in the client, not a boundary here (VATUSA/OIS#474).
pub async fn airport_demand(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuProgramRead>,
) -> Result<Json<Vec<crate::models::AirportDemandBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    // All DB access happens here, before any feed work — `feed::*` has no pool (AGENTS.md).
    let (programs, configs, cfrs) = tokio::try_join!(
        tmu_repo::list_programs(pool),
        config_repo::list_all(pool),
        tmu_repo::all_issued_cfrs(pool),
    )?;

    let programs: HashMap<String, ProgramInputs> = programs
        .into_iter()
        .map(|p| {
            (
                p.icao.clone(),
                ProgramInputs {
                    aar: p.aar,
                    trail: p.trail,
                    mit: p.mit,
                    gates: p
                        .gates
                        .0
                        .iter()
                        .map(|g| flow::GateSpacing {
                            name: g.name.clone(),
                            trail: g.trail,
                            mit: g.mit,
                        })
                        .collect(),
                    exclude_wake: p.exclude_wake,
                    exclude_types: p.exclude_types,
                    jets_only: p.jets_only,
                },
            )
        })
        .collect();

    let mut configs_by_icao: HashMap<String, Vec<crate::models::AirportConfigBody>> =
        HashMap::new();
    for c in configs {
        configs_by_icao.entry(c.icao.clone()).or_default().push(c);
    }

    // One fold, replacing a per-airport `issued_cfr_map` call each.
    let mut issued_by_airport: HashMap<String, HashMap<String, DateTime<Utc>>> = HashMap::new();
    for (callsign, airport, wheels_up) in cfrs {
        issued_by_airport
            .entry(airport)
            .or_default()
            .insert(callsign, wheels_up);
    }

    // Clone the snapshot + airport handles under one brief lock, then release it before the CPU.
    let (snapshot, airports) = {
        let guard = state.feed.read().await;
        (guard.snapshot.clone(), guard.airports.clone())
    };
    let Some(snap) = snapshot else {
        return Ok(Json(Vec::new()));
    };

    let nav = state.nav.load_full();
    let winds = state.winds.load_full();
    let profiles = state.aircraft_profiles.load_full();
    let gates = state.gates.load_full();
    let runways = state.runways.clone();
    let taxi_estimate_samples = state.taxi_estimate_samples.load_full();
    let manual_exclusions =
        crate::handlers::flow::all_excluded_callsigns(state.flight_exclusions.load().as_ref());
    let now = Utc::now();

    let rows = tokio::task::spawn_blocking(move || {
        // Only airports something is actually flying to can be over capacity, so intersect the
        // capacity-bearing set with the arrivals in this snapshot. One pass.
        let mut arriving: HashSet<String> = HashSet::new();
        for fp in snap
            .data
            .pilots
            .iter()
            .filter_map(|p| p.flight_plan.as_ref())
            .chain(
                snap.data
                    .prefiles
                    .iter()
                    .filter_map(|p| p.flight_plan.as_ref()),
            )
        {
            arriving.insert(fp.arrival.to_ascii_uppercase());
        }

        let mut rows: Vec<crate::models::AirportDemandBody> = programs
            .keys()
            .chain(configs_by_icao.keys())
            .collect::<HashSet<_>>()
            .into_iter()
            .filter(|icao| arriving.contains(*icao))
            .filter_map(|icao| {
                let program = programs.get(icao);
                let empty: Vec<crate::models::AirportConfigBody> = Vec::new();
                let configs = configs_by_icao.get(icao).unwrap_or(&empty);
                let (aar, aar_source) = effective_aar(program, configs)?;

                let empty_issued = HashMap::new();
                let flow = flow::compute(
                    icao,
                    program,
                    &snap.data,
                    airports.as_ref(),
                    nav.as_ref(),
                    winds.as_ref(),
                    profiles.as_ref(),
                    issued_by_airport.get(icao).unwrap_or(&empty_issued),
                    gates.as_ref(),
                    runways.as_ref(),
                    taxi_estimate_samples.as_ref(),
                    &manual_exclusions,
                    now,
                );

                let demand_60min = flow.demand_60min as i32;
                Some(crate::models::AirportDemandBody {
                    icao: icao.clone(),
                    demand_60min,
                    aar,
                    exceedance: demand_60min - aar,
                    aar_source: aar_source.to_string(),
                    inbound: flow.inbound as i32,
                    airborne: flow.airborne as i32,
                    ground: flow.ground as i32,
                })
            })
            .collect();

        // Rank server-side, so the ordering is right even for a client that doesn't sort.
        rows.sort_by(|a, b| {
            b.exceedance
                .cmp(&a.exceedance)
                .then_with(|| a.icao.cmp(&b.icao))
        });
        rows
    })
    .await
    .map_err(|_| ApiError::Internal)?;

    Ok(Json(rows))
}

#[utoipa::path(
    get,
    path = "/api/v1/tmu/taxi/{icao}",
    tag = "tmu",
    params(("icao" = String, Path, description = "Airport ICAO")),
    responses((status = 200, body = crate::feed::taxi::TaxiField), (status = 401))
)]
pub async fn taxi_stats(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuProgramRead>,
    Path(icao): Path<String>,
) -> Json<taxi::TaxiField> {
    let icao = icao.trim().to_ascii_uppercase();
    let now = Utc::now();
    let guard = state.feed.read().await;
    let empty = crate::feed::vatsim::VatsimData::default();
    let data = guard.snapshot.as_ref().map(|s| &s.data).unwrap_or(&empty);
    Json(taxi::field(
        &guard.taxi_sessions,
        guard.taxi_samples.get(&icao),
        data,
        &icao,
        now,
    ))
}

#[utoipa::path(
    get,
    path = "/api/v1/tmu/departures/{dep}",
    tag = "tmu",
    params(("dep" = String, Path, description = "Departure field: airport, TRACON, or ARTCC")),
    responses((status = 200, body = crate::models::DeparturesResponse), (status = 401), (status = 503))
)]
pub async fn list_departures(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuProgramRead>,
    Path(dep): Path<String>,
) -> Result<Json<DeparturesResponse>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let dep = dep.trim().to_ascii_uppercase();
    // Read the live snapshot, then release the lock before computing per-destination flows.
    let snap = state
        .feed
        .read()
        .await
        .snapshot
        .clone()
        .unwrap_or_else(|| Arc::new(crate::feed::Snapshot::of(Default::default())));
    let winds = state.winds.load_full();
    Ok(Json(
        departures_response(&state, pool, &dep, snap, winds, Utc::now()).await?,
    ))
}

/// Build the departure-field CFR view for `dep` against a given snapshot at `now`. Shared by the
/// live handler and the historical replay.
pub(crate) async fn departures_response(
    state: &AppState,
    pool: &PgPool,
    dep: &str,
    snap: Arc<crate::feed::Snapshot>,
    winds: Arc<crate::feed::winds::Winds>,
    now: DateTime<Utc>,
) -> Result<DeparturesResponse, ApiError> {
    // Resolve the field: an airport is just itself; a TRACON/ARTCC spans many airports.
    let (facility_kind, mut airports) = {
        let map = state.facilities.read().await;
        let kind = map.get(dep).map(|f| f.kind.clone());
        (kind, facilities::member_airports(&map, dep))
    };
    airports.sort();
    let member_set: HashSet<String> = airports.iter().cloned().collect();

    let metered: HashSet<String> = tmu_repo::list_programs(pool)
        .await?
        .into_iter()
        .map(|p| p.icao)
        .collect();

    let pending = flow::pending_departures(&member_set, &snap.data);

    // FCA-issued releases (RDY/RLSD) for these departures — so a release set on the FCA page also
    // shows here, even when the destination has no GDP program (KSAN metered by an FCA, not a GDP).
    let pending_callsigns: Vec<String> = pending.iter().map(|d| d.callsign.clone()).collect();
    let fca_releases = flow_repo::releases_for_callsigns(pool, &pending_callsigns).await?;

    // Metering data (by callsign) for destinations that have a program.
    let dests: HashSet<String> = pending
        .iter()
        .map(|d| d.arrival.clone())
        .filter(|a| metered.contains(a))
        .collect();
    let mut meta: HashMap<String, MeteredCfr> = HashMap::new();
    for dest in &dests {
        let flow = flow_from_data(state, pool, dest, snap.clone(), winds.clone(), now).await?;
        for f in flow.flights {
            meta.insert(
                f.callsign,
                MeteredCfr {
                    eta: f.eta,
                    sta: f.sta,
                    delay_min: f.delay_min,
                    cfr: f.cfr,
                    cfr_issued: f.cfr_issued,
                    seq: f.seq,
                },
            );
        }
    }

    let mut departures: Vec<DepartureFlight> = pending
        .into_iter()
        .map(|d| {
            let m = meta.remove(&d.callsign).unwrap_or_default();
            // An FCA release counts as a (frozen) CFR too. Prefer the GDP-program CFR when present;
            // otherwise fall back to the FCA's release time and mark the flight metered.
            let fca_cfr = fca_releases
                .get(&d.callsign)
                .and_then(|ms| DateTime::from_timestamp_millis(*ms));
            let cfr = m.cfr.or(fca_cfr);
            let has_program = metered.contains(&d.arrival) || fca_cfr.is_some();
            DepartureFlight {
                callsign: d.callsign,
                dep: d.dep,
                arrival: d.arrival,
                aircraft_type: d.aircraft_type,
                gate: d.gate,
                status: d.status,
                has_program,
                eta: m.eta,
                sta: m.sta,
                delay_min: m.delay_min,
                cfr,
                cfr_issued: m.cfr_issued || fca_cfr.is_some(),
                seq: m.seq,
            }
        })
        .collect();
    // Metered (with a CFR) first, ordered by release; unmetered fall to the bottom.
    departures.sort_by_key(|r| r.cfr.map(|c| c.timestamp_millis()).unwrap_or(i64::MAX));

    let total = departures.len();
    let to_metered = departures.iter().filter(|r| r.has_program).count();
    let holding_on_cfr = departures
        .iter()
        .filter(|r| r.has_program && (r.cfr_issued || r.delay_min > 0))
        .count();
    let mut program_destinations: Vec<String> = dests.into_iter().collect();
    program_destinations.sort();

    Ok(DeparturesResponse {
        facility_kind,
        airports,
        total,
        to_metered,
        holding_on_cfr,
        program_destinations,
        departures,
    })
}

/// Per-callsign metering outputs pulled from a destination's computed flow.
#[derive(Default)]
struct MeteredCfr {
    eta: Option<DateTime<Utc>>,
    sta: Option<DateTime<Utc>>,
    delay_min: i64,
    cfr: Option<DateTime<Utc>>,
    cfr_issued: bool,
    seq: Option<i64>,
}

#[utoipa::path(
    post,
    path = "/api/v1/tmu/cfr",
    tag = "tmu",
    request_body = IssueCfrRequest,
    responses((status = 200, body = IssuedCfrBody), (status = 400), (status = 401))
)]
pub async fn issue_cfr(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuCfrAssign>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Json(payload): Json<IssueCfrRequest>,
) -> Result<Json<IssuedCfrBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    let callsign = payload.callsign.trim().to_ascii_uppercase();
    let airport = payload.airport.trim().to_ascii_uppercase();
    if callsign.is_empty() || airport.len() < 3 {
        return Err(ApiError::BadRequest);
    }

    // With a ready time, lock the closest open runway slot at or after it. Otherwise
    // lock the flight's currently proposed wheels-up (or now if it has none).
    let wheels_up = match payload.ready_time {
        Some(ready) => {
            let program = program_inputs(pool, &airport).await?;
            let issued = tmu_repo::issued_cfr_map(pool, &airport).await?;
            let (snapshot, airports) = {
                let guard = state.feed.read().await;
                (guard.snapshot.clone(), guard.airports.clone())
            };
            match (program, snapshot) {
                (Some(pg), Some(snap)) => {
                    let nav = state.nav.load_full();
                    let winds = state.winds.load_full();
                    let profiles = state.aircraft_profiles.load_full();
                    let gates = state.gates.load_full();
                    let runways = state.runways.clone();
                    let taxi_estimate_samples = state.taxi_estimate_samples.load_full();
                    let airport = airport.clone();
                    let callsign = callsign.clone();
                    let manual_exclusions = crate::handlers::flow::all_excluded_callsigns(
                        state.flight_exclusions.load().as_ref(),
                    );
                    // Route-resolving CPU — keep it off the async runtime (see `flow_from_data`).
                    tokio::task::spawn_blocking(move || {
                        flow::ready_time_slot(
                            &airport,
                            &pg,
                            &snap.data,
                            airports.as_ref(),
                            nav.as_ref(),
                            winds.as_ref(),
                            profiles.as_ref(),
                            &issued,
                            gates.as_ref(),
                            runways.as_ref(),
                            taxi_estimate_samples.as_ref(),
                            &manual_exclusions,
                            &callsign,
                            ready,
                            Utc::now(),
                        )
                    })
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or(ready)
                }
                _ => ready,
            }
        }
        None => {
            let flow = flow_for(&state, pool, &airport).await?;
            flow.flights
                .iter()
                .find(|f| f.callsign == callsign)
                .and_then(|f| f.cfr)
                .unwrap_or_else(Utc::now)
        }
    };

    tmu_repo::upsert_issued_cfr(pool, &callsign, &airport, wheels_up, &user.id).await?;
    state.publish(crate::realtime::topic::CFR);
    let _ = tmu_repo::prune_stale_cfrs(pool).await;
    let cfr = tmu_repo::get_issued_cfr(pool, &callsign)
        .await?
        .ok_or(ApiError::Internal)?;
    Ok(Json(cfr))
}

#[utoipa::path(
    delete,
    path = "/api/v1/tmu/cfr/{callsign}",
    tag = "tmu",
    params(("callsign" = String, Path, description = "Flight callsign")),
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn release_cfr(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuCfrAssign>,
    Path(callsign): Path<String>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let callsign = callsign.trim().to_ascii_uppercase();
    if !tmu_repo::delete_issued_cfr(pool, &callsign).await? {
        return Err(ApiError::NotFound);
    }
    state.publish(crate::realtime::topic::CFR);
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::AirportConfigBody;

    fn config(name: &str, aar: i32, calm_default: bool) -> AirportConfigBody {
        AirportConfigBody {
            id: name.to_string(),
            icao: "KDCA".to_string(),
            name: name.to_string(),
            aar,
            adr: 40,
            landing_runways: vec!["01".to_string()],
            wind_from_deg: 0,
            wind_to_deg: 90,
            calm_default,
            artcc: "ZDC".to_string(),
            updated_at: Utc::now(),
            updated_by: None,
            editable: false,
        }
    }

    fn program(aar: i32) -> ProgramInputs {
        ProgramInputs {
            aar,
            trail: 0,
            mit: 0,
            gates: Vec::new(),
            exclude_wake: Vec::new(),
            exclude_types: Vec::new(),
            jets_only: false,
        }
    }

    // --- Which capacity a demand figure is ranked against (VATUSA/OIS#475) ---

    #[test]
    fn a_metering_program_outranks_the_runway_config() {
        // The program AAR is the rate actually metering traffic, so it must win even when a config
        // exists — ranking against the config would contradict the sequence controllers are working.
        let configs = vec![config("north", 60, true)];
        assert_eq!(
            effective_aar(Some(&program(30)), &configs),
            Some((30, "program"))
        );
    }

    #[test]
    fn without_a_program_it_falls_back_to_the_calm_default_config() {
        // `None` wind deliberately skips the favored-config lookup, which would cost one outbound
        // forecast request per airport.
        let configs = vec![config("north", 44, false), config("calm", 52, true)];
        assert_eq!(effective_aar(None, &configs), Some((52, "config")));
    }

    #[test]
    fn without_a_calm_default_it_takes_the_first_config() {
        let configs = vec![config("north", 44, false), config("south", 48, false)];
        assert_eq!(effective_aar(None, &configs), Some((44, "config")));
    }

    #[test]
    fn an_airport_with_neither_is_left_out_rather_than_ranked() {
        assert_eq!(effective_aar(None, &[]), None);
    }

    // --- Through the router ---
    //
    // `effective_aar` above stays green if the handler stops gating, so these send real requests
    // with a real session: `RequirePermission<TmuProgramRead>` is on the tested path.

    use crate::scope_test_support::{self, artcc, grant, send, session_cookie, test_state};
    use http::{Method, StatusCode};

    const DEMAND: &str = "/api/v1/tmu/demand";

    fn state_for(pool: PgPool) -> AppState {
        test_state(pool, HashMap::from([("ZDC".to_string(), artcc(&["KDCA"]))]))
    }

    #[sqlx::test]
    async fn without_the_permission_the_ranking_is_401(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        let cookie = session_cookie(&pool, &user).await;
        let state = state_for(pool);
        assert_eq!(
            send(&state, Method::GET, DEMAND, &cookie, None).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[sqlx::test]
    async fn a_national_reader_gets_the_ranking(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "tmu.program.read", None).await;
        let cookie = session_cookie(&pool, &user).await;
        let state = state_for(pool);
        assert_eq!(
            send(&state, Method::GET, DEMAND, &cookie, None).await,
            StatusCode::OK
        );
    }

    #[sqlx::test]
    async fn a_facility_scoped_reader_also_gets_it(pool: PgPool) {
        // Deliberate: the gate is coarse, matching the sibling feed reads. The ranking is derived
        // from public VATSIM traffic, so "national" is menu curation in the client (VATUSA/OIS#474),
        // not a boundary here. If this ever becomes 403, that was a policy change — not a tidy-up.
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "tmu.program.read", Some("ZDC")).await;
        let cookie = session_cookie(&pool, &user).await;
        let state = state_for(pool);
        assert_eq!(
            send(&state, Method::GET, DEMAND, &cookie, None).await,
            StatusCode::OK
        );
    }
}
