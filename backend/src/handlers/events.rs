//! Event handlers — read the VATUSA event cache that anchors per-event planning.

use axum::{
    Json,
    extract::{Extension, Path, State},
};

use crate::{
    auth::{
        context::CurrentUser,
        permissions::{EventsPlanRead, EventsPlanUpdate},
        require_permission::RequirePermission,
    },
    errors::ApiError,
    models::{
        DccRequestBody, EventBody, FacilitySupportBody, UpdateDccRequest,
        UpsertFacilitySupportRequest,
    },
    repos::events as events_repo,
    state::AppState,
};

const DCC_STATUSES: [&str; 3] = ["not_needed", "requested", "confirmed"];
const SUPPORT_LEVELS: [&str; 3] = ["required", "preferred", "not_required"];

fn normalize_facility(raw: &str) -> Option<String> {
    let f = raw.trim().to_ascii_uppercase();
    (!f.is_empty() && f.len() <= 8 && f.chars().all(|c| c.is_ascii_alphanumeric())).then_some(f)
}

fn default_dcc() -> DccRequestBody {
    DccRequestBody {
        status: "not_needed".to_string(),
        notes: String::new(),
        updated_at: None,
        updated_by: None,
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/events",
    tag = "events",
    responses((status = 200, body = Vec<EventBody>), (status = 401))
)]
pub async fn list_events(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
) -> Result<Json<Vec<EventBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(events_repo::list_upcoming(pool).await?))
}

#[utoipa::path(
    get,
    path = "/api/v1/events/{id}",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    responses((status = 200, body = EventBody), (status = 401), (status = 404))
)]
pub async fn get_event(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Path(id): Path<i64>,
) -> Result<Json<EventBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    events_repo::get(pool, id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

#[utoipa::path(
    get,
    path = "/api/v1/events/{id}/dcc",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    responses((status = 200, body = DccRequestBody), (status = 401))
)]
pub async fn get_event_dcc(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Path(id): Path<i64>,
) -> Result<Json<DccRequestBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(
        events_repo::get_dcc(pool, id)
            .await?
            .unwrap_or_else(default_dcc),
    ))
}

#[utoipa::path(
    put,
    path = "/api/v1/events/{id}/dcc",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    request_body = UpdateDccRequest,
    responses((status = 200, body = DccRequestBody), (status = 400), (status = 401), (status = 404))
)]
pub async fn update_event_dcc(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateDccRequest>,
) -> Result<Json<DccRequestBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    if !DCC_STATUSES.contains(&payload.status.as_str()) {
        return Err(ApiError::BadRequest);
    }
    if events_repo::get(pool, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }

    let notes = payload.notes.unwrap_or_default();
    events_repo::upsert_dcc(pool, id, &payload.status, notes.trim(), &user.id).await?;
    Ok(Json(
        events_repo::get_dcc(pool, id)
            .await?
            .unwrap_or_else(default_dcc),
    ))
}

#[utoipa::path(
    get,
    path = "/api/v1/events/{id}/facilities",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    responses((status = 200, body = Vec<FacilitySupportBody>), (status = 401))
)]
pub async fn list_event_facilities(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<FacilitySupportBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(events_repo::list_facility_support(pool, id).await?))
}

#[utoipa::path(
    put,
    path = "/api/v1/events/{id}/facilities/{facility}",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("facility" = String, Path, description = "ARTCC/TRACON id")
    ),
    request_body = UpsertFacilitySupportRequest,
    responses((status = 200, body = FacilitySupportBody), (status = 400), (status = 401), (status = 404))
)]
pub async fn upsert_event_facility(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path((id, facility)): Path<(i64, String)>,
    Json(payload): Json<UpsertFacilitySupportRequest>,
) -> Result<Json<FacilitySupportBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    let facility = normalize_facility(&facility).ok_or(ApiError::BadRequest)?;
    if !SUPPORT_LEVELS.contains(&payload.level.as_str()) {
        return Err(ApiError::BadRequest);
    }
    if events_repo::get(pool, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }

    let notes = payload.notes.unwrap_or_default();
    events_repo::upsert_facility_support(
        pool,
        id,
        &facility,
        &payload.level,
        notes.trim(),
        &user.id,
    )
    .await?;
    events_repo::get_facility_support(pool, id, &facility)
        .await?
        .map(Json)
        .ok_or(ApiError::Internal)
}

#[utoipa::path(
    delete,
    path = "/api/v1/events/{id}/facilities/{facility}",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("facility" = String, Path, description = "ARTCC/TRACON id")
    ),
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn delete_event_facility(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Path((id, facility)): Path<(i64, String)>,
) -> Result<axum::http::StatusCode, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let facility = normalize_facility(&facility).ok_or(ApiError::BadRequest)?;
    if events_repo::delete_facility_support(pool, id, &facility).await? {
        Ok(axum::http::StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}
