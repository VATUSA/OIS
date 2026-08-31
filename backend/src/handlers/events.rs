//! Event handlers — read the VATUSA event cache that anchors per-event planning.

use axum::{
    Json,
    extract::{Extension, Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    auth::{
        context::{CurrentApiKey, CurrentUser},
        permissions::{
            EventsDebriefCreate, EventsDiscordPublish, EventsPlanRead, EventsPlanUpdate,
            EventsRateUpdate, EventsSupportUpdate, StatsCaptureUpdate,
        },
        principal::Principal,
        require_permission::RequirePermission,
    },
    errors::ApiError,
    feed,
    models::{
        AddPackageItemRequest, AirportRateBody, AirportStatBody, CombinedStatBody,
        CreateGroundStopRequest, CreatePackageRequest, CreateTmiRequest, DccRequestBody,
        EventAvailabilityBody, EventBody, EventCaptureBody, EventDebriefBody, EventStatsBody,
        FacilitySupportBody, FcaBody, KeyCountBody, SetFcaAutoRequest, TmiPackageBody,
        UpdateDccRequest, UpdateEventCaptureRequest, UpdateEventDebriefRequest,
        UpsertAirportRateRequest, UpsertFacilitySupportRequest, UpsertFcaRequest,
        UpsertProgramRequest,
    },
    repos::{
        access as access_repo, ace as ace_repo, availability as availability_repo,
        events as events_repo, flow as flow_repo, integration as integration_repo,
        stats as stats_repo, tmu as tmu_repo,
    },
    state::AppState,
};

// --- TMI-package item payloads (the create-shape per kind) ---

#[derive(Debug, Deserialize, Serialize)]
struct ProgramItem {
    icao: String,
    aar: i32,
    #[serde(default)]
    trail: i32,
    #[serde(default)]
    mit: i32,
}

#[derive(Debug, Deserialize, Serialize)]
struct RestrictionItem {
    requesting: String,
    providing: String,
    restriction: String,
    #[serde(default)]
    start_time: Option<DateTime<Utc>>,
    #[serde(default)]
    stop_time: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize, Serialize)]
struct GroundStopItem {
    airport: String,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    until: Option<String>,
}

/// Validate + normalize a package item's payload for its kind (canonical for storage).
fn normalize_item(kind: &str, payload: Value) -> Result<Value, ApiError> {
    match kind {
        "program" => {
            let mut p: ProgramItem =
                serde_json::from_value(payload).map_err(|_| ApiError::BadRequest)?;
            p.icao = normalize_icao(&p.icao).ok_or(ApiError::BadRequest)?;
            if !(0..=200).contains(&p.aar) {
                return Err(ApiError::BadRequest);
            }
            p.trail = p.trail.max(0);
            p.mit = p.mit.max(0);
            serde_json::to_value(p).map_err(|_| ApiError::Internal)
        }
        "restriction" => {
            let mut r: RestrictionItem =
                serde_json::from_value(payload).map_err(|_| ApiError::BadRequest)?;
            r.requesting = r.requesting.trim().to_string();
            r.providing = r.providing.trim().to_string();
            r.restriction = r.restriction.trim().to_string();
            if r.requesting.is_empty() || r.providing.is_empty() || r.restriction.is_empty() {
                return Err(ApiError::BadRequest);
            }
            serde_json::to_value(r).map_err(|_| ApiError::Internal)
        }
        "ground_stop" => {
            let mut g: GroundStopItem =
                serde_json::from_value(payload).map_err(|_| ApiError::BadRequest)?;
            g.airport = normalize_icao(&g.airport).ok_or(ApiError::BadRequest)?;
            g.scope = g
                .scope
                .map(|s| s.trim().to_ascii_uppercase())
                .filter(|s| !s.is_empty());
            g.until = g
                .until
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            serde_json::to_value(g).map_err(|_| ApiError::Internal)
        }
        _ => Err(ApiError::BadRequest),
    }
}

const DCC_STATUSES: [&str; 3] = ["not_needed", "requested", "confirmed"];
const SUPPORT_LEVELS: [&str; 3] = ["required", "preferred", "not_required"];
const RATE_PERMISSION: &str = "events.rate.update";
const SUPPORT_PERMISSION: &str = "events.support.update";

fn normalize_facility(raw: &str) -> Option<String> {
    let f = raw.trim().to_ascii_uppercase();
    (!f.is_empty() && f.len() <= 8 && f.chars().all(|c| c.is_ascii_alphanumeric())).then_some(f)
}

pub(crate) fn normalize_icao(raw: &str) -> Option<String> {
    let s = raw.trim().to_ascii_uppercase();
    (s.len() >= 3 && s.len() <= 4 && s.chars().all(|c| c.is_ascii_alphanumeric())).then_some(s)
}

/// The ARTCC that owns `icao`, from the live facility map (None if unknown).
pub(crate) async fn owning_artcc(state: &AppState, icao: &str) -> Option<String> {
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
    Ok(Json(events_repo::list_all(pool).await?))
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
    responses((status = 200, body = Vec<FacilitySupportBody>), (status = 401), (status = 404))
)]
pub async fn list_event_facilities(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<FacilitySupportBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let event = events_repo::get(pool, id)
        .await?
        .ok_or(ApiError::NotFound)?;

    // Derive the involved facilities from data we already have, unioned with any saved rows:
    //   host ARTCC (event.facility) · owning ARTCCs of configured airports · ACE staffing requests.
    let stored: std::collections::HashMap<String, FacilitySupportBody> =
        events_repo::list_facility_support(pool, id)
            .await?
            .into_iter()
            .map(|r| (r.facility.clone(), r))
            .collect();

    // ARTCC -> its configured airports on this event.
    let mut airports_by_facility: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for r in events_repo::list_airport_rates(pool, id).await? {
        if !r.artcc.is_empty() {
            airports_by_facility
                .entry(r.artcc)
                .or_default()
                .push(r.icao);
        }
    }

    // `has_staffing` now means "this ARTCC has an open ACE request on the event" (the old
    // events.staffing_request source was replaced by event-scoped ACE requests).
    let staffing: std::collections::HashSet<String> = ace_repo::open_request_artccs(pool, id)
        .await?
        .into_iter()
        .collect();

    let host = normalize_facility(&event.facility);

    // Union of every facility id that any signal (or a saved row) surfaced.
    let mut ids: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    ids.extend(stored.keys().cloned());
    ids.extend(airports_by_facility.keys().cloned());
    ids.extend(staffing.iter().cloned());
    if let Some(h) = host.as_ref() {
        ids.insert(h.clone());
    }

    // Editability is facility-scoped on events.support.update.
    let principal = Principal::optional(current_user.as_ref(), current_api_key.as_ref());
    let scope = match principal.as_ref() {
        Some(p) => Some(p.permission_scope(&state, SUPPORT_PERMISSION).await?),
        None => None,
    };

    let rows: Vec<FacilitySupportBody> = ids
        .into_iter()
        .map(|facility| {
            let is_host = host.as_deref() == Some(facility.as_str());
            let mut airports = airports_by_facility
                .get(&facility)
                .cloned()
                .unwrap_or_default();
            airports.sort();
            let has_staffing = staffing.contains(&facility);
            let editable = scope
                .as_ref()
                .map(|s| s.allows(Some(facility.as_str())))
                .unwrap_or(false);

            match stored.get(&facility) {
                Some(row) => FacilitySupportBody {
                    facility: facility.clone(),
                    level: row.level.clone(),
                    notes: row.notes.clone(),
                    updated_at: row.updated_at,
                    updated_by: row.updated_by.clone(),
                    is_host,
                    airports,
                    has_staffing,
                    stored: true,
                    editable,
                },
                // Derived-only suggestion: default host to required, everything else to preferred.
                None => FacilitySupportBody {
                    facility,
                    level: if is_host { "required" } else { "preferred" }.to_string(),
                    notes: String::new(),
                    updated_at: None,
                    updated_by: None,
                    is_host,
                    airports,
                    has_staffing,
                    stored: false,
                    editable,
                },
            }
        })
        .collect();

    Ok(Json(rows))
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
    _permission: RequirePermission<EventsSupportUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path((id, facility)): Path<(i64, String)>,
    Json(payload): Json<UpsertFacilitySupportRequest>,
) -> Result<Json<FacilitySupportBody>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    let facility = normalize_facility(&facility).ok_or(ApiError::BadRequest)?;
    if !SUPPORT_LEVELS.contains(&payload.level.as_str()) {
        return Err(ApiError::BadRequest);
    }
    if events_repo::get(pool, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }

    // Facility scope: the caller must hold events.support.update nationally or for this facility.
    let scope = principal
        .permission_scope(&state, SUPPORT_PERMISSION)
        .await?;
    if !scope.allows(Some(facility.as_str())) {
        return Err(ApiError::Forbidden);
    }

    let notes = payload.notes.unwrap_or_default();
    events_repo::upsert_facility_support(
        pool,
        id,
        &facility,
        &payload.level,
        notes.trim(),
        principal.user_id(),
    )
    .await?;
    let mut row = events_repo::get_facility_support(pool, id, &facility)
        .await?
        .ok_or(ApiError::Internal)?;
    row.stored = true;
    row.editable = true;
    Ok(Json(row))
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
    _permission: RequirePermission<EventsSupportUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path((id, facility)): Path<(i64, String)>,
) -> Result<axum::http::StatusCode, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let facility = normalize_facility(&facility).ok_or(ApiError::BadRequest)?;

    // Facility scope: the caller must hold events.support.update nationally or for this facility.
    let scope = principal
        .permission_scope(&state, SUPPORT_PERMISSION)
        .await?;
    if !scope.allows(Some(facility.as_str())) {
        return Err(ApiError::Forbidden);
    }

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
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<AirportRateBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let mut rates = events_repo::list_airport_rates(pool, id).await?;

    // Mark each row editable per the caller's ARTCC scope for events.rate.update.
    if let Some(principal) = Principal::optional(current_user.as_ref(), current_api_key.as_ref()) {
        let scope = principal.permission_scope(&state, RATE_PERMISSION).await?;
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
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path((id, icao)): Path<(i64, String)>,
    Json(payload): Json<UpsertAirportRateRequest>,
) -> Result<Json<AirportRateBody>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
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
    let scope = principal.permission_scope(&state, RATE_PERMISSION).await?;
    if !scope.allows(artcc.as_deref()) {
        return Err(ApiError::Forbidden);
    }

    let source = match payload.source.as_deref() {
        Some("predicted") => "predicted",
        _ => "override",
    };
    events_repo::upsert_airport_rate(
        pool,
        id,
        &icao,
        payload.aar,
        payload.adr,
        artcc.as_deref().unwrap_or(""),
        payload.config_id.as_deref(),
        source,
        principal.user_id(),
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
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path((id, icao)): Path<(i64, String)>,
) -> Result<axum::http::StatusCode, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    let existing = events_repo::get_airport_rate(pool, id, &icao)
        .await?
        .ok_or(ApiError::NotFound)?;

    // Scope-check against the ARTCC recorded on the row.
    let artcc = (!existing.artcc.is_empty()).then_some(existing.artcc.as_str());
    let scope = principal.permission_scope(&state, RATE_PERMISSION).await?;
    if !scope.allows(artcc) {
        return Err(ApiError::Forbidden);
    }

    events_repo::delete_airport_rate(pool, id, &icao).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// --- TMI packages ---

/// Confirm a package exists and belongs to `event_id`; returns its status.
async fn package_status(
    pool: &sqlx::PgPool,
    event_id: i64,
    package_id: &str,
) -> Result<String, ApiError> {
    let (owner_event, status) = events_repo::get_package_owner(pool, package_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if owner_event != event_id {
        return Err(ApiError::NotFound);
    }
    Ok(status)
}

#[utoipa::path(
    get,
    path = "/api/v1/events/{id}/packages",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    responses((status = 200, body = Vec<TmiPackageBody>), (status = 401))
)]
pub async fn list_event_packages(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<TmiPackageBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(events_repo::list_packages(pool, id).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/events/{id}/packages",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    request_body = CreatePackageRequest,
    responses((status = 200, body = Vec<TmiPackageBody>), (status = 400), (status = 401), (status = 404))
)]
pub async fn create_event_package(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<i64>,
    Json(payload): Json<CreatePackageRequest>,
) -> Result<Json<Vec<TmiPackageBody>>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let name = payload.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest);
    }
    if events_repo::get(pool, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    events_repo::create_package(pool, id, name, &user.id).await?;
    Ok(Json(events_repo::list_packages(pool, id).await?))
}

#[utoipa::path(
    delete,
    path = "/api/v1/events/{id}/packages/{package_id}",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("package_id" = String, Path, description = "Package id")
    ),
    responses((status = 200, body = Vec<TmiPackageBody>), (status = 401), (status = 404))
)]
pub async fn delete_event_package(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Path((id, package_id)): Path<(i64, String)>,
) -> Result<Json<Vec<TmiPackageBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    package_status(pool, id, &package_id).await?;
    events_repo::delete_package(pool, &package_id).await?;
    Ok(Json(events_repo::list_packages(pool, id).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/events/{id}/packages/{package_id}/items",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("package_id" = String, Path, description = "Package id")
    ),
    request_body = AddPackageItemRequest,
    responses((status = 200, body = Vec<TmiPackageBody>), (status = 400), (status = 401), (status = 404), (status = 409))
)]
pub async fn add_event_package_item(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Path((id, package_id)): Path<(i64, String)>,
    Json(payload): Json<AddPackageItemRequest>,
) -> Result<Json<Vec<TmiPackageBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if package_status(pool, id, &package_id).await? != "draft" {
        return Err(ApiError::Conflict); // can't edit an activated package
    }
    let canonical = normalize_item(&payload.kind, payload.payload)?;
    events_repo::add_package_item(pool, &package_id, &payload.kind, &canonical).await?;
    Ok(Json(events_repo::list_packages(pool, id).await?))
}

#[utoipa::path(
    delete,
    path = "/api/v1/events/{id}/packages/{package_id}/items/{item_id}",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("package_id" = String, Path, description = "Package id"),
        ("item_id" = String, Path, description = "Item id")
    ),
    responses((status = 200, body = Vec<TmiPackageBody>), (status = 401), (status = 404))
)]
pub async fn delete_event_package_item(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Path((id, package_id, item_id)): Path<(i64, String, String)>,
) -> Result<Json<Vec<TmiPackageBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    package_status(pool, id, &package_id).await?;
    if !events_repo::delete_package_item(pool, &package_id, &item_id).await? {
        return Err(ApiError::NotFound);
    }
    Ok(Json(events_repo::list_packages(pool, id).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/events/{id}/packages/{package_id}/activate",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("package_id" = String, Path, description = "Package id")
    ),
    responses((status = 200, body = Vec<TmiPackageBody>), (status = 401), (status = 404), (status = 409))
)]
pub async fn activate_event_package(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path((id, package_id)): Path<(i64, String)>,
) -> Result<Json<Vec<TmiPackageBody>>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    if package_status(pool, id, &package_id).await? != "draft" {
        return Err(ApiError::Conflict); // already activated
    }
    let event = events_repo::get(pool, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let items = events_repo::list_package_items(pool, &package_id).await?;

    // A published restriction posts to Discord like any other; resolve the channel once (None ⇒ skip).
    let tmu_channel = integration_repo::channel_id(pool, crate::handlers::tmu::TMU_CHANNEL).await?;

    // Materialize each draft item into the live TMU tables, recording a `live_ref` so the package
    // can later be deactivated (cancelling exactly what it created).
    for item in &items {
        let payload = item.payload.0.clone();
        let live_ref = match item.kind.as_str() {
            "program" => {
                let p: ProgramItem =
                    serde_json::from_value(payload).map_err(|_| ApiError::Internal)?;
                let req = UpsertProgramRequest {
                    aar: p.aar,
                    trail: p.trail,
                    mit: p.mit,
                    gates: Vec::new(),
                    exclude_wake: Vec::new(),
                    exclude_types: Vec::new(),
                    jets_only: false,
                    // planned programs auto-expire an hour after the event ends
                    active_until: Some(event.end_time),
                };
                tmu_repo::upsert_program(pool, &p.icao, &req, &[], &user.id).await?;
                // Programs are keyed by ICAO; that's the handle for later cleanup.
                p.icao
            }
            "restriction" => {
                let r: RestrictionItem =
                    serde_json::from_value(payload).map_err(|_| ApiError::Internal)?;
                let req = CreateTmiRequest {
                    requesting: r.requesting,
                    providing: r.providing,
                    restriction: r.restriction,
                    start_time: r.start_time,
                    stop_time: r.stop_time,
                };
                // Activation goes live: create then publish so the restriction is active.
                let tmi_id = tmu_repo::create_tmi(pool, &req, &user.id).await?;
                let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
                let tmi = tmu_repo::publish_tmi(&mut tx, &tmi_id, &user.id).await?;
                if let (Some(tmi), Some(channel_id)) = (tmi, tmu_channel.clone()) {
                    let job = serde_json::json!({
                        "channel_id": channel_id,
                        "tmi_id": tmi.id,
                        "requesting": tmi.requesting,
                        "providing": tmi.providing,
                        "restriction": tmi.restriction,
                        "start_time": tmi.start_time,
                        "stop_time": tmi.stop_time,
                    });
                    integration_repo::enqueue_job(
                        &mut tx,
                        "tmi_publish",
                        &job,
                        Some("tmi"),
                        Some(tmi.id.as_str()),
                    )
                    .await?;
                }
                tx.commit().await.map_err(|_| ApiError::Internal)?;
                tmi_id
            }
            "ground_stop" => {
                let g: GroundStopItem =
                    serde_json::from_value(payload).map_err(|_| ApiError::Internal)?;
                let scope = g.scope.clone().unwrap_or_default();
                let req = CreateGroundStopRequest {
                    airport: g.airport.clone(),
                    scope: g.scope.clone(),
                    until: g.until.clone(),
                };
                let gs_id =
                    tmu_repo::create_ground_stop(pool, &req, &scope, g.until.as_deref(), &user.id)
                        .await?;
                tmu_repo::publish_ground_stop(pool, &gs_id, &user.id).await?;
                gs_id
            }
            _ => continue,
        };
        events_repo::set_item_live_ref(pool, &item.id, &live_ref).await?;
    }

    events_repo::mark_package_activated(pool, &package_id, &user.id).await?;
    Ok(Json(events_repo::list_packages(pool, id).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/events/{id}/packages/{package_id}/deactivate",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("package_id" = String, Path, description = "Package id")
    ),
    responses((status = 200, body = Vec<TmiPackageBody>), (status = 401), (status = 404), (status = 409))
)]
pub async fn deactivate_event_package(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path((id, package_id)): Path<(i64, String)>,
) -> Result<Json<Vec<TmiPackageBody>>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    if package_status(pool, id, &package_id).await? != "activated" {
        return Err(ApiError::Conflict); // only an activated package can be deactivated
    }

    // Cancel exactly the live rows this package created (best-effort — a row already cleared
    // manually or auto-expired just returns false), then archive the package.
    for item in events_repo::list_package_item_refs(pool, &package_id).await? {
        let Some(reference) = item.live_ref.as_deref() else {
            continue;
        };
        match item.kind.as_str() {
            "program" => {
                tmu_repo::delete_program(pool, reference).await?;
            }
            "restriction" => {
                tmu_repo::cancel_tmi(pool, reference).await?;
            }
            "ground_stop" => {
                tmu_repo::cancel_ground_stop(pool, reference).await?;
            }
            _ => {}
        }
    }

    events_repo::mark_package_archived(pool, &package_id, &user.id).await?;
    Ok(Json(events_repo::list_packages(pool, id).await?))
}

// --- per-event stats capture + generated stats ---------------------------------------------------

const CAPTURE_PERMISSION: &str = "stats.capture.update";

/// Build the capture-config body (config defaults + current capture status).
async fn capture_body(
    pool: &sqlx::PgPool,
    event_id: i64,
    can_edit: bool,
) -> Result<EventCaptureBody, ApiError> {
    let cfg = stats_repo::get_event_capture(pool, event_id).await?;
    let cap = stats_repo::latest_capture_for_event(pool, event_id).await?;
    Ok(EventCaptureBody {
        enabled: cfg.as_ref().is_some_and(|c| c.enabled),
        pre_minutes: cfg.as_ref().map_or(30, |c| c.pre_minutes),
        post_minutes: cfg.as_ref().map_or(30, |c| c.post_minutes),
        updated_at: cfg.as_ref().map(|c| c.updated_at),
        updated_by: cfg.and_then(|c| c.updated_by),
        capture_status: cap.as_ref().map(|c| c.status.clone()),
        capture_id: cap.as_ref().map(|c| c.id.clone()),
        capture_start: cap.as_ref().map(|c| c.start_time),
        capture_end: cap.and_then(|c| c.end_time),
        can_edit,
    })
}

// --- Event-specific FCAs (planned in the event manager; stored in flow.fca with an event_id) ---

/// Fetch an FCA and confirm it belongs to `event_id` — 404 otherwise, so one event can't touch
/// another's (or a shared) FCA through these routes.
async fn owned_event_fca(
    pool: &sqlx::PgPool,
    event_id: i64,
    fca_id: &str,
) -> Result<FcaBody, ApiError> {
    let fca = flow_repo::get_fca(pool, fca_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if fca.event_id != Some(event_id) {
        return Err(ApiError::NotFound);
    }
    Ok(fca)
}

#[utoipa::path(
    get,
    path = "/api/v1/events/{id}/fcas",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    responses((status = 200, body = Vec<FcaBody>), (status = 401), (status = 404))
)]
pub async fn list_event_fcas(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<FcaBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(flow_repo::list_event_fcas(pool, id).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/events/{id}/fcas",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    request_body = UpsertFcaRequest,
    responses((status = 200, body = Vec<FcaBody>), (status = 400), (status = 401), (status = 404))
)]
pub async fn create_event_fca(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<i64>,
    Json(payload): Json<UpsertFcaRequest>,
) -> Result<Json<Vec<FcaBody>>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if payload.name.trim().is_empty() || payload.points.len() < 2 {
        return Err(ApiError::BadRequest);
    }
    if events_repo::get(pool, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    flow_repo::create_event_fca(pool, id, &payload, &user.id).await?;
    Ok(Json(flow_repo::list_event_fcas(pool, id).await?))
}

#[utoipa::path(
    put,
    path = "/api/v1/events/{id}/fcas/{fca_id}",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("fca_id" = String, Path, description = "FCA id")
    ),
    request_body = UpsertFcaRequest,
    responses((status = 200, body = Vec<FcaBody>), (status = 400), (status = 401), (status = 404))
)]
pub async fn update_event_fca(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path((id, fca_id)): Path<(i64, String)>,
    Json(payload): Json<UpsertFcaRequest>,
) -> Result<Json<Vec<FcaBody>>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if payload.name.trim().is_empty() || payload.points.len() < 2 {
        return Err(ApiError::BadRequest);
    }
    owned_event_fca(pool, id, &fca_id).await?;
    flow_repo::update_fca(pool, &fca_id, &payload, &user.id).await?;
    Ok(Json(flow_repo::list_event_fcas(pool, id).await?))
}

#[utoipa::path(
    delete,
    path = "/api/v1/events/{id}/fcas/{fca_id}",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("fca_id" = String, Path, description = "FCA id")
    ),
    responses((status = 200, body = Vec<FcaBody>), (status = 401), (status = 404))
)]
pub async fn delete_event_fca(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Path((id, fca_id)): Path<(i64, String)>,
) -> Result<Json<Vec<FcaBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    owned_event_fca(pool, id, &fca_id).await?;
    flow_repo::delete_fca(pool, &fca_id).await?;
    Ok(Json(flow_repo::list_event_fcas(pool, id).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/events/{id}/fcas/{fca_id}/publish",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("fca_id" = String, Path, description = "FCA id")
    ),
    responses((status = 200, body = Vec<FcaBody>), (status = 401), (status = 404), (status = 409))
)]
pub async fn publish_event_fca(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Path((id, fca_id)): Path<(i64, String)>,
) -> Result<Json<Vec<FcaBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let fca = owned_event_fca(pool, id, &fca_id).await?;
    if fca.event_status.as_deref() != Some("planned") {
        return Err(ApiError::Conflict); // only a planned FCA can be published
    }
    flow_repo::mark_event_fca_published(pool, id, &fca_id).await?;
    state.publish(crate::realtime::topic::FCA); // it's live on every map now
    Ok(Json(flow_repo::list_event_fcas(pool, id).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/events/{id}/fcas/{fca_id}/archive",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("fca_id" = String, Path, description = "FCA id")
    ),
    responses((status = 200, body = Vec<FcaBody>), (status = 401), (status = 404), (status = 409))
)]
pub async fn archive_event_fca(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Path((id, fca_id)): Path<(i64, String)>,
) -> Result<Json<Vec<FcaBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let fca = owned_event_fca(pool, id, &fca_id).await?;
    if !matches!(fca.event_status.as_deref(), Some("planned" | "published")) {
        return Err(ApiError::Conflict); // already archived
    }
    flow_repo::mark_event_fca_archived(pool, id, &fca_id).await?;
    state.publish(crate::realtime::topic::FCA); // archiving a published FCA removes it from live maps
    Ok(Json(flow_repo::list_event_fcas(pool, id).await?))
}

#[utoipa::path(
    put,
    path = "/api/v1/events/{id}/fcas/{fca_id}/auto",
    tag = "events",
    params(
        ("id" = i64, Path, description = "VATUSA event id"),
        ("fca_id" = String, Path, description = "FCA id")
    ),
    request_body = SetFcaAutoRequest,
    responses((status = 200, body = Vec<FcaBody>), (status = 401), (status = 404))
)]
pub async fn set_event_fca_auto(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanUpdate>,
    Path((id, fca_id)): Path<(i64, String)>,
    Json(payload): Json<SetFcaAutoRequest>,
) -> Result<Json<Vec<FcaBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    owned_event_fca(pool, id, &fca_id).await?;
    flow_repo::set_event_fca_auto(pool, id, &fca_id, payload.auto_publish).await?;
    Ok(Json(flow_repo::list_event_fcas(pool, id).await?))
}

#[utoipa::path(
    get,
    path = "/api/v1/events/{id}/capture",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    responses((status = 200, body = EventCaptureBody), (status = 401), (status = 404))
)]
pub async fn get_event_capture(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path(id): Path<i64>,
) -> Result<Json<EventCaptureBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if events_repo::get(pool, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    let can_edit = match Principal::optional(current_user.as_ref(), current_api_key.as_ref()) {
        Some(principal) => !principal
            .permission_scope(&state, CAPTURE_PERMISSION)
            .await?
            .is_empty(),
        None => false,
    };
    Ok(Json(capture_body(pool, id, can_edit).await?))
}

#[utoipa::path(
    put,
    path = "/api/v1/events/{id}/capture",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    request_body = UpdateEventCaptureRequest,
    responses((status = 200, body = EventCaptureBody), (status = 400), (status = 401), (status = 404))
)]
pub async fn update_event_capture(
    State(state): State<AppState>,
    _permission: RequirePermission<StatsCaptureUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateEventCaptureRequest>,
) -> Result<Json<EventCaptureBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if events_repo::get(pool, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    let pre = payload.pre_minutes.unwrap_or(30).clamp(0, 720);
    let post = payload.post_minutes.unwrap_or(30).clamp(0, 720);
    stats_repo::upsert_event_capture(pool, id, payload.enabled, pre, post, &user.id).await?;
    Ok(Json(capture_body(pool, id, true).await?))
}

#[utoipa::path(
    get,
    path = "/api/v1/events/{id}/availability",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    responses((status = 200, body = Vec<EventAvailabilityBody>), (status = 401))
)]
pub async fn get_event_availability(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<EventAvailabilityBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    availability_repo::list_for_event(pool, id).await.map(Json)
}

#[utoipa::path(
    get,
    path = "/api/v1/events/{id}/stats",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    responses((status = 200, body = EventStatsBody), (status = 401), (status = 404))
)]
pub async fn get_event_stats(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Path(id): Path<i64>,
) -> Result<Json<EventStatsBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if events_repo::get(pool, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }

    let empty_combined = || CombinedStatBody {
        arrivals: 0,
        departures: 0,
        movements: 0,
        unique_pilots: 0,
        top_aircraft: Vec::new(),
    };

    let Some(cap) = stats_repo::latest_capture_for_event(pool, id).await? else {
        return Ok(Json(EventStatsBody {
            captured: false,
            status: None,
            window_start: None,
            window_end: None,
            airports: Vec::new(),
            combined: empty_combined(),
        }));
    };

    let from = cap.start_time;
    let to = cap.end_time.unwrap_or_else(Utc::now);

    // Featured airports = the event's configured (rated) airports.
    let icaos: Vec<String> = events_repo::list_airport_rates(pool, id)
        .await?
        .into_iter()
        .map(|r| r.icao)
        .collect();

    if icaos.is_empty() {
        return Ok(Json(EventStatsBody {
            captured: true,
            status: Some(cap.status),
            window_start: Some(from),
            window_end: Some(to),
            airports: Vec::new(),
            combined: empty_combined(),
        }));
    }

    let key_count = |k: stats_repo::KeyCount| KeyCountBody {
        key: k.key,
        count: k.count,
    };

    // Per-airport breakdown + top aircraft (grouped by ICAO).
    let breakdown = stats_repo::event_airport_breakdown(pool, &icaos, from, to).await?;
    let mut top_by_icao: std::collections::HashMap<String, Vec<KeyCountBody>> =
        std::collections::HashMap::new();
    for r in stats_repo::event_airport_top_aircraft(pool, &icaos, from, to, 4).await? {
        top_by_icao.entry(r.icao).or_default().push(KeyCountBody {
            key: r.key,
            count: r.count,
        });
    }

    let airports: Vec<AirportStatBody> = breakdown
        .into_iter()
        .map(|b| AirportStatBody {
            movements: b.arrivals + b.departures,
            top_aircraft: top_by_icao.remove(&b.icao).unwrap_or_default(),
            icao: b.icao,
            arrivals: b.arrivals,
            departures: b.departures,
            unique_pilots: b.unique_pilots,
        })
        .collect();

    // Combined: movements sum across airports; pilots deduped; top aircraft across all.
    let arrivals: i64 = airports.iter().map(|a| a.arrivals).sum();
    let departures: i64 = airports.iter().map(|a| a.departures).sum();
    let combined = CombinedStatBody {
        arrivals,
        departures,
        movements: arrivals + departures,
        unique_pilots: stats_repo::event_combined_unique_pilots(pool, &icaos, from, to).await?,
        top_aircraft: stats_repo::event_combined_top_aircraft(pool, &icaos, from, to)
            .await?
            .into_iter()
            .map(key_count)
            .collect(),
    };

    Ok(Json(EventStatsBody {
        captured: true,
        status: Some(cap.status),
        window_start: Some(from),
        window_end: Some(to),
        airports,
        combined,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/events/{id}/debrief",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    responses((status = 200, body = EventDebriefBody), (status = 401))
)]
pub async fn get_event_debrief(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<i64>,
) -> Result<Json<EventDebriefBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let editable = match current_user.as_ref() {
        Some(u) => access_repo::fetch_user_permission_names(pool, &u.id)
            .await?
            .iter()
            .any(|p| p == "events.debrief.create"),
        None => false,
    };
    let (notes, updated_by, updated_at) = match events_repo::get_debrief(pool, id).await? {
        Some((n, by, at)) => (n, by, Some(at)),
        None => (String::new(), None, None),
    };
    Ok(Json(EventDebriefBody {
        notes,
        updated_by,
        updated_at,
        editable,
    }))
}

#[utoipa::path(
    put,
    path = "/api/v1/events/{id}/debrief",
    tag = "events",
    params(("id" = i64, Path, description = "VATUSA event id")),
    request_body = UpdateEventDebriefRequest,
    responses((status = 200, body = EventDebriefBody), (status = 400), (status = 401), (status = 404))
)]
pub async fn update_event_debrief(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsDebriefCreate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateEventDebriefRequest>,
) -> Result<Json<EventDebriefBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if payload.notes.len() > 20_000 {
        return Err(ApiError::BadRequest);
    }
    if events_repo::get(pool, id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    events_repo::upsert_debrief(pool, id, &payload.notes, &user.id).await?;
    let (notes, updated_by, updated_at) = match events_repo::get_debrief(pool, id).await? {
        Some((n, by, at)) => (n, by, Some(at)),
        None => (payload.notes, None, None),
    };
    Ok(Json(EventDebriefBody {
        notes,
        updated_by,
        updated_at,
        editable: true,
    }))
}

/// Fallback channel (a config logical name) when the host has no region / no region channel is mapped.
const EVENTS_CHANNEL: &str = "events";

/// Host ARTCC → DCC region id (from the VATUSA portal's event-region grouping, trimmed to the real
/// facility set). The region id is the suffix of the `region-{id}` channel logical name.
fn dcc_region(facility: &str) -> Option<&'static str> {
    match facility.to_ascii_uppercase().as_str() {
        "ZBW" | "ZDC" | "ZNY" | "ZOB" => Some("northeast"),
        "ZID" | "ZJX" | "ZMA" | "ZTL" => Some("southeast"),
        "ZAB" | "ZFW" | "ZHU" | "ZME" => Some("southcentral"),
        "ZAU" | "ZDV" | "ZKC" | "ZMP" => Some("midwest"),
        "ZAN" | "HCF" | "ZLA" | "ZLC" | "ZOA" | "ZSE" => Some("west"),
        _ => None,
    }
}

#[utoipa::path(
    post, path = "/api/v1/events/{id}/discord/publish", tag = "events",
    params(("id" = i64, Path)),
    responses(
        (status = 202, description = "Thread creation enqueued"),
        (status = 400, description = "No Discord channel configured for this region"),
        (status = 401), (status = 404),
        (status = 409, description = "Already posted for this event")
    )
)]
pub async fn publish_event_discord(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsDiscordPublish>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let event = events_repo::get(pool, id)
        .await?
        .ok_or(ApiError::NotFound)?;

    let subject_id = id.to_string();
    // Don't create a second thread if one was already posted (the bot acked a create for this event).
    if integration_repo::succeeded_job_result(pool, "event", &subject_id, "event_thread_create")
        .await?
        .is_some()
    {
        return Err(ApiError::Conflict);
    }

    // Route by the host's DCC region; fall back to the generic `events` channel if unmapped/unconfigured.
    let mut channel = None;
    if let Some(region) = dcc_region(&event.facility) {
        channel = integration_repo::channel_id(pool, &format!("region-{region}")).await?;
    }
    if channel.is_none() {
        channel = integration_repo::channel_id(pool, EVENTS_CHANNEL).await?;
    }
    let channel = channel.ok_or(ApiError::BadRequest)?;

    // Involved facilities = Required/Preferred support rows; each pings the facility's EC(s) — the OIS
    // users holding the `EC` role scoped to that ARTCC (set in Access Control), by their linked Discord.
    let mut facilities = Vec::new();
    for f in events_repo::list_facility_support(pool, id)
        .await?
        .into_iter()
        .filter(|f| matches!(f.level.as_str(), "required" | "preferred"))
    {
        let ec_user_ids = integration_repo::ec_discord_ids(pool, &f.facility).await?;
        facilities.push(serde_json::json!({ "id": f.facility, "ec_user_ids": ec_user_ids }));
    }
    let ntmo_role_id = integration_repo::role_id(pool, "ntmo").await?;
    let dcc_trainee_role_id = integration_repo::role_id(pool, "dcc-trainee").await?;

    let thread_name = format!("{} {}", event.start_time.format("%Y%m%d"), event.title);
    let date_line = format!(
        "{} · {}z",
        event.start_time.format("%a, %b %-d"),
        event.start_time.format("%H%M")
    );

    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    let payload = serde_json::json!({
        "channel_id": channel,
        "thread_name": thread_name,
        "event_title": event.title,
        "date_line": date_line,
        "facilities": facilities,
        "ntmo_role_id": ntmo_role_id,
        "dcc_trainee_role_id": dcc_trainee_role_id,
        "event_id": id,
    });
    integration_repo::enqueue_job(
        &mut tx,
        "event_thread_create",
        &payload,
        Some("event"),
        Some(&subject_id),
    )
    .await?;
    tx.commit().await.map_err(|_| ApiError::Internal)?;
    Ok(StatusCode::ACCEPTED)
}
