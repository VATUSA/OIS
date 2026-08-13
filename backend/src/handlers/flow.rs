//! Flow handlers — FCA CRUD + a lightweight live-traffic feed for the FCA map.

use axum::{
    Json,
    extract::{Extension, Path, State},
    http::StatusCode,
};

use crate::{
    auth::{
        context::CurrentUser,
        permissions::{FlowFcaDelete, FlowFcaRead, FlowFcaUpdate},
        require_permission::RequirePermission,
    },
    errors::ApiError,
    models::{FcaBody, TrafficAircraft, UpsertFcaRequest},
    repos::flow as flow_repo,
    state::AppState,
};

fn validate_fca(req: &UpsertFcaRequest) -> Result<(), ApiError> {
    if req.name.trim().is_empty() || req.points.len() < 2 {
        return Err(ApiError::BadRequest);
    }
    if let Some(m) = &req.mode {
        if m != "rate" && m != "mit" {
            return Err(ApiError::BadRequest);
        }
    }
    Ok(())
}

#[utoipa::path(
    get,
    path = "/api/v1/flow/fcas",
    tag = "flow",
    responses((status = 200, body = Vec<FcaBody>), (status = 401))
)]
pub async fn list_fcas(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaRead>,
) -> Result<Json<Vec<FcaBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(flow_repo::list_fcas(pool).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/flow/fcas",
    tag = "flow",
    request_body = UpsertFcaRequest,
    responses((status = 200, body = FcaBody), (status = 400), (status = 401))
)]
pub async fn create_fca(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Json(payload): Json<UpsertFcaRequest>,
) -> Result<Json<FcaBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    validate_fca(&payload)?;
    let id = flow_repo::create_fca(pool, &payload, &user.id).await?;
    flow_repo::get_fca(pool, &id)
        .await?
        .map(Json)
        .ok_or(ApiError::Internal)
}

#[utoipa::path(
    put,
    path = "/api/v1/flow/fcas/{id}",
    tag = "flow",
    params(("id" = String, Path, description = "FCA id")),
    request_body = UpsertFcaRequest,
    responses((status = 200, body = FcaBody), (status = 400), (status = 401), (status = 404))
)]
pub async fn update_fca(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
    Json(payload): Json<UpsertFcaRequest>,
) -> Result<Json<FcaBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    validate_fca(&payload)?;
    if !flow_repo::update_fca(pool, &id, &payload, &user.id).await? {
        return Err(ApiError::NotFound);
    }
    flow_repo::get_fca(pool, &id)
        .await?
        .map(Json)
        .ok_or(ApiError::Internal)
}

#[utoipa::path(
    delete,
    path = "/api/v1/flow/fcas/{id}",
    tag = "flow",
    params(("id" = String, Path, description = "FCA id")),
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn delete_fca(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaDelete>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if flow_repo::delete_fca(pool, &id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/flow/traffic",
    tag = "flow",
    responses((status = 200, body = Vec<TrafficAircraft>), (status = 401))
)]
pub async fn list_traffic(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaRead>,
) -> Json<Vec<TrafficAircraft>> {
    let guard = state.feed.read().await;
    let aircraft = guard
        .snapshot
        .as_ref()
        .map(|snap| {
            snap.data
                .pilots
                .iter()
                .filter(|p| p.latitude != 0.0 || p.longitude != 0.0)
                .map(|p| {
                    let fp = p.flight_plan.as_ref();
                    TrafficAircraft {
                        callsign: p.callsign.clone(),
                        lat: p.latitude,
                        lon: p.longitude,
                        heading: p.heading,
                        gs: p.groundspeed,
                        alt: p.altitude,
                        dep: fp.map(|f| f.departure.clone()).unwrap_or_default(),
                        arr: fp.map(|f| f.arrival.clone()).unwrap_or_default(),
                        actype: fp.map(|f| f.aircraft_short.clone()).unwrap_or_default(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    Json(aircraft)
}
