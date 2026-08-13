//! TMU handlers — Traffic Management Initiatives (TMIs).

use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;

use crate::{
    auth::{
        context::CurrentUser,
        permissions::{TmuTmiCreate, TmuTmiDelete, TmuTmiPublish, TmuTmiRead, TmuTmiUpdate},
        require_permission::RequirePermission,
    },
    errors::ApiError,
    models::{CreateTmiRequest, TmiBody, UpdateTmiRequest},
    repos::{org as org_repo, tmu as tmu_repo},
    state::AppState,
};

#[derive(Deserialize)]
pub struct TmiListQuery {
    status: Option<String>,
}

async fn validate_artcc(
    pool: &sqlx::PgPool,
    artcc_id: Option<&str>,
) -> Result<Option<String>, ApiError> {
    match artcc_id.map(str::trim).filter(|v| !v.is_empty()) {
        None => Ok(None),
        Some(id) => {
            let id = id.to_ascii_uppercase();
            if org_repo::find_facility(pool, &id).await?.is_none() {
                return Err(ApiError::BadRequest);
            }
            Ok(Some(id))
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/tmu/tmis",
    tag = "tmu",
    params(("status" = Option<String>, Query, description = "Filter by status")),
    responses((status = 200, body = Vec<TmiBody>), (status = 401))
)]
pub async fn list_tmis(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuTmiRead>,
    Query(query): Query<TmiListQuery>,
) -> Result<Json<Vec<TmiBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let status = query
        .status
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    Ok(Json(tmu_repo::list_tmis(pool, status).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/tmu/tmis",
    tag = "tmu",
    request_body = CreateTmiRequest,
    responses((status = 200, body = TmiBody), (status = 400), (status = 401))
)]
pub async fn create_tmi(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuTmiCreate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Json(mut payload): Json<CreateTmiRequest>,
) -> Result<Json<TmiBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    if payload.kind.trim().is_empty()
        || payload.element.trim().is_empty()
        || payload.restriction.trim().is_empty()
    {
        return Err(ApiError::BadRequest);
    }
    payload.artcc_id = validate_artcc(pool, payload.artcc_id.as_deref()).await?;

    let id = tmu_repo::create_tmi(pool, &payload, &user.id).await?;
    let tmi = tmu_repo::get_tmi(pool, &id)
        .await?
        .ok_or(ApiError::Internal)?;
    Ok(Json(tmi))
}

#[utoipa::path(
    patch,
    path = "/api/v1/tmu/tmis/{id}",
    tag = "tmu",
    params(("id" = String, Path, description = "TMI id")),
    request_body = UpdateTmiRequest,
    responses((status = 200, body = TmiBody), (status = 400), (status = 401), (status = 404))
)]
pub async fn update_tmi(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuTmiUpdate>,
    Path(id): Path<String>,
    Json(mut payload): Json<UpdateTmiRequest>,
) -> Result<Json<TmiBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if payload.artcc_id.is_some() {
        payload.artcc_id = validate_artcc(pool, payload.artcc_id.as_deref()).await?;
    }
    if !tmu_repo::update_tmi(pool, &id, &payload).await? {
        return Err(ApiError::NotFound);
    }
    let tmi = tmu_repo::get_tmi(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(tmi))
}

#[utoipa::path(
    post,
    path = "/api/v1/tmu/tmis/{id}/publish",
    tag = "tmu",
    params(("id" = String, Path, description = "TMI id")),
    responses((status = 200, body = TmiBody), (status = 401), (status = 409))
)]
pub async fn publish_tmi(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuTmiPublish>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
) -> Result<Json<TmiBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if !tmu_repo::publish_tmi(pool, &id, &user.id).await? {
        return Err(ApiError::Conflict); // not a draft (or absent)
    }
    let tmi = tmu_repo::get_tmi(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(tmi))
}

#[utoipa::path(
    post,
    path = "/api/v1/tmu/tmis/{id}/cancel",
    tag = "tmu",
    params(("id" = String, Path, description = "TMI id")),
    responses((status = 200, body = TmiBody), (status = 401), (status = 409))
)]
pub async fn cancel_tmi(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuTmiPublish>,
    Path(id): Path<String>,
) -> Result<Json<TmiBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if !tmu_repo::cancel_tmi(pool, &id).await? {
        return Err(ApiError::Conflict);
    }
    let tmi = tmu_repo::get_tmi(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(tmi))
}

#[utoipa::path(
    delete,
    path = "/api/v1/tmu/tmis/{id}",
    tag = "tmu",
    params(("id" = String, Path, description = "TMI id")),
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn delete_tmi(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuTmiDelete>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if !tmu_repo::delete_tmi(pool, &id).await? {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}
