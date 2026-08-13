//! Event handlers — read the VATUSA event cache that anchors per-event planning.

use axum::{
    Json,
    extract::{Extension, Path, State},
};

use crate::{
    auth::{
        context::CurrentUser,
        permissions::{EventsPlanRead, EventsPlanUpdate, EventsRateUpdate, EventsStaffingCreate},
        require_permission::RequirePermission,
    },
    errors::ApiError,
    feed,
    models::{
        AirportRateBody, DccRequestBody, EventBody, FacilitySupportBody, StaffingRequestBody,
        UpdateDccRequest, UpsertAirportRateRequest, UpsertFacilitySupportRequest,
        UpsertStaffingRequest,
    },
    repos::{access as access_repo, events as events_repo},
    state::AppState,
};

const DCC_STATUSES: [&str; 3] = ["not_needed", "requested", "confirmed"];
const SUPPORT_LEVELS: [&str; 3] = ["required", "preferred", "not_required"];
const STAFFING_STATUSES: [&str; 3] = ["open", "met", "closed"];
const RATE_PERMISSION: &str = "events.rate.update";

fn normalize_facility(raw: &str) -> Option<String> {
    let f = raw.trim().to_ascii_uppercase();
    (!f.is_empty() && f.len() <= 8 && f.chars().all(|c| c.is_ascii_alphanumeric())).then_some(f)
}

fn normalize_icao(raw: &str) -> Option<String> {
    let s = raw.trim().to_ascii_uppercase();
    (s.len() >= 3 && s.len() <= 4 && s.chars().all(|c| c.is_ascii_alphanumeric())).then_some(s)
}

/// The ARTCC that owns `icao`, from the live facility map (None if unknown).
async fn owning_artcc(state: &AppState, icao: &str) -> Option<String> {
    let map = state.facilities.read().await;
    feed::facilities::artcc_for_airport(&map, icao)
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

#[utoipa::path(
    get,
    path = "/api/v1/events/{id}/rates",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    responses((status = 200, body = Vec<AirportRateBody>), (status = 401))
)]
pub async fn list_event_rates(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<AirportRateBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let mut rates = events_repo::list_airport_rates(pool, id).await?;

    // Mark each row editable per the caller's ARTCC scope for events.rate.update.
    if let Some(user) = current_user.as_ref() {
        let scope = access_repo::permission_scope(pool, &user.id, RATE_PERMISSION).await?;
        for r in rates.iter_mut() {
            let artcc = (!r.artcc.is_empty()).then_some(r.artcc.as_str());
            r.editable = scope.allows(artcc);
        }
    }
    Ok(Json(rates))
}

#[utoipa::path(
    put,
    path = "/api/v1/events/{id}/rates/{icao}",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("icao" = String, Path, description = "Airport ICAO")
    ),
    request_body = UpsertAirportRateRequest,
    responses((status = 200, body = AirportRateBody), (status = 400), (status = 401), (status = 403), (status = 404))
)]
pub async fn upsert_event_rate(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsRateUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path((id, icao)): Path<(i64, String)>,
    Json(payload): Json<UpsertAirportRateRequest>,
) -> Result<Json<AirportRateBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    if !(0..=200).contains(&payload.aar) || !(0..=200).contains(&payload.adr) {
        return Err(ApiError::BadRequest);
    }
    if events_repo::get(pool, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }

    // Facility scope: the caller must hold events.rate.update nationally or for the
    // airport's owning ARTCC.
    let artcc = owning_artcc(&state, &icao).await;
    let scope = access_repo::permission_scope(pool, &user.id, RATE_PERMISSION).await?;
    if !scope.allows(artcc.as_deref()) {
        return Err(ApiError::Forbidden);
    }

    events_repo::upsert_airport_rate(
        pool,
        id,
        &icao,
        payload.aar,
        payload.adr,
        artcc.as_deref().unwrap_or(""),
        &user.id,
    )
    .await?;
    let mut row = events_repo::get_airport_rate(pool, id, &icao)
        .await?
        .ok_or(ApiError::Internal)?;
    row.editable = true;
    Ok(Json(row))
}

#[utoipa::path(
    delete,
    path = "/api/v1/events/{id}/rates/{icao}",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("icao" = String, Path, description = "Airport ICAO")
    ),
    responses((status = 204), (status = 401), (status = 403), (status = 404))
)]
pub async fn delete_event_rate(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsRateUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path((id, icao)): Path<(i64, String)>,
) -> Result<axum::http::StatusCode, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    let existing = events_repo::get_airport_rate(pool, id, &icao)
        .await?
        .ok_or(ApiError::NotFound)?;

    // Scope-check against the ARTCC recorded on the row.
    let artcc = (!existing.artcc.is_empty()).then_some(existing.artcc.as_str());
    let scope = access_repo::permission_scope(pool, &user.id, RATE_PERMISSION).await?;
    if !scope.allows(artcc) {
        return Err(ApiError::Forbidden);
    }

    events_repo::delete_airport_rate(pool, id, &icao).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/v1/events/{id}/staffing",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    responses((status = 200, body = Vec<StaffingRequestBody>), (status = 401))
)]
pub async fn list_event_staffing(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<StaffingRequestBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(events_repo::list_staffing(pool, id).await?))
}

#[utoipa::path(
    put,
    path = "/api/v1/events/{id}/staffing/{facility}",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("facility" = String, Path, description = "ARTCC id")
    ),
    request_body = UpsertStaffingRequest,
    responses((status = 200, body = StaffingRequestBody), (status = 400), (status = 401), (status = 404))
)]
pub async fn upsert_event_staffing(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsStaffingCreate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path((id, facility)): Path<(i64, String)>,
    Json(payload): Json<UpsertStaffingRequest>,
) -> Result<Json<StaffingRequestBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    let facility = normalize_facility(&facility).ok_or(ApiError::BadRequest)?;
    if !STAFFING_STATUSES.contains(&payload.status.as_str())
        || !(0..=999).contains(&payload.positions_requested)
        || !(0..=999).contains(&payload.positions_filled)
    {
        return Err(ApiError::BadRequest);
    }
    if events_repo::get(pool, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }

    let notes = payload.notes.unwrap_or_default();
    events_repo::upsert_staffing(
        pool,
        id,
        &facility,
        payload.positions_requested,
        payload.positions_filled,
        &payload.status,
        notes.trim(),
        &user.id,
    )
    .await?;
    events_repo::get_staffing(pool, id, &facility)
        .await?
        .map(Json)
        .ok_or(ApiError::Internal)
}

#[utoipa::path(
    delete,
    path = "/api/v1/events/{id}/staffing/{facility}",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("facility" = String, Path, description = "ARTCC id")
    ),
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn delete_event_staffing(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsStaffingCreate>,
    Path((id, facility)): Path<(i64, String)>,
) -> Result<axum::http::StatusCode, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let facility = normalize_facility(&facility).ok_or(ApiError::BadRequest)?;
    if events_repo::delete_staffing(pool, id, &facility).await? {
        Ok(axum::http::StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}
