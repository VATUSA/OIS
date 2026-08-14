//! Live-feed handlers: feed health, per-airport arrival flow (metered against a program),
//! the departure-field CFR view, and issuing/releasing CFRs.

use std::collections::{HashMap, HashSet};

use axum::{
    Json,
    extract::{Extension, Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use utoipa::ToSchema;

use crate::{
    auth::{
        context::CurrentUser,
        permissions::{TmuCfrAssign, TmuProgramRead},
        require_permission::RequirePermission,
    },
    errors::ApiError,
    feed::facilities,
    feed::flow::{self, ProgramInputs},
    feed::taxi,
    models::{DepartureFlight, DeparturesResponse, IssueCfrRequest, IssuedCfrBody},
    repos::tmu as tmu_repo,
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
async fn program_inputs(pool: &PgPool, icao: &str) -> Result<Option<ProgramInputs>, ApiError> {
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

/// Compute the live, metered flow for one arrival airport (loads program + issued CFRs).
async fn flow_for(state: &AppState, pool: &PgPool, icao: &str) -> Result<flow::Flow, ApiError> {
    let program = program_inputs(pool, icao).await?;
    let issued = tmu_repo::issued_cfr_map(pool, icao).await?;
    let guard = state.feed.read().await;
    let flow = match &guard.snapshot {
        Some(snap) => flow::compute(
            icao,
            program.as_ref(),
            &snap.data,
            &guard.airports,
            state.winds.load_full().as_ref(),
            &issued,
            Utc::now(),
        ),
        None => flow::Flow {
            icao: icao.to_string(),
            aar: program.map(|p| p.aar),
            inbound: 0,
            airborne: 0,
            ground: 0,
            proposed: 0,
            demand_60min: 0,
            over_capacity: None,
            flights: Vec::new(),
        },
    };
    Ok(flow)
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

    // Resolve the field: an airport is just itself; a TRACON/ARTCC spans many airports.
    let (facility_kind, mut airports) = {
        let map = state.facilities.read().await;
        let kind = map.get(&dep).map(|f| f.kind.clone());
        (kind, facilities::member_airports(&map, &dep))
    };
    airports.sort();
    let member_set: HashSet<String> = airports.iter().cloned().collect();

    let metered: HashSet<String> = tmu_repo::list_programs(pool)
        .await?
        .into_iter()
        .map(|p| p.icao)
        .collect();

    // Every pending departure out of the resolved airports (read the snapshot, then release
    // the lock before computing per-destination flows).
    let pending = {
        let guard = state.feed.read().await;
        match &guard.snapshot {
            Some(snap) => flow::pending_departures(&member_set, &snap.data),
            None => Vec::new(),
        }
    };

    // Metering data (by callsign) for destinations that have a program.
    let dests: HashSet<String> = pending
        .iter()
        .map(|d| d.arrival.clone())
        .filter(|a| metered.contains(a))
        .collect();
    let mut meta: HashMap<String, MeteredCfr> = HashMap::new();
    for dest in &dests {
        let flow = flow_for(&state, pool, dest).await?;
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
            let has_program = metered.contains(&d.arrival);
            let m = meta.remove(&d.callsign).unwrap_or_default();
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
                cfr: m.cfr,
                cfr_issued: m.cfr_issued,
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

    Ok(Json(DeparturesResponse {
        facility_kind,
        airports,
        total,
        to_metered,
        holding_on_cfr,
        program_destinations,
        departures,
    }))
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
            let guard = state.feed.read().await;
            match (program.as_ref(), guard.snapshot.as_ref()) {
                (Some(pg), Some(snap)) => flow::ready_time_slot(
                    &airport,
                    pg,
                    &snap.data,
                    &guard.airports,
                    state.winds.load_full().as_ref(),
                    &issued,
                    &callsign,
                    ready,
                    Utc::now(),
                )
                .unwrap_or(ready),
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
    Ok(StatusCode::NO_CONTENT)
}
