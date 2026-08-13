//! Live-feed handlers: feed health + per-airport arrival flow (metered against a program).

use axum::{
    Json,
    extract::{Path, State},
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

use crate::{
    auth::{permissions::TmuProgramRead, require_permission::RequirePermission},
    errors::ApiError,
    feed::flow::{self, ProgramInputs},
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

    let program = tmu_repo::get_program(pool, &icao)
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
        });

    let guard = state.feed.read().await;
    let flow = match &guard.snapshot {
        Some(snap) => flow::compute(
            &icao,
            program.as_ref(),
            &snap.data,
            &guard.airports,
            Utc::now(),
        ),
        // No feed yet — return an empty picture rather than erroring.
        None => flow::Flow {
            icao,
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
    Ok(Json(flow))
}
