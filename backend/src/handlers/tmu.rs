//! TMU handlers — Traffic Management Initiatives (TMIs).

use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::{
    auth::{
        context::CurrentUser,
        permissions::{
            TmuGroundStopCreate, TmuGroundStopDelete, TmuGroundStopPublish, TmuGroundStopRead,
            TmuProgramDelete, TmuProgramRead, TmuProgramUpdate, TmuTmiCreate, TmuTmiDelete,
            TmuTmiPublish, TmuTmiRead, TmuTmiUpdate,
        },
        require_permission::RequirePermission,
    },
    errors::ApiError,
    models::{
        CreateGroundStopRequest, CreateTmiRequest, GateRule, GroundStopBody, ProgramBody, TmiBody,
        UpdateTmiRequest, UpsertProgramRequest,
    },
    repos::{integration as integration_repo, tmu as tmu_repo},
    state::AppState,
};
use serde_json::json;

/// Logical channel name (mapped to a snowflake in the Discord config) where published TMIs are posted.
pub(crate) const TMU_CHANNEL: &str = "tmu-advisories";

#[derive(Debug, Default, Deserialize)]
pub struct TmiListQuery {
    status: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    facility: Option<String>,
    /// Active-during range (RFC 3339); a TMI matches when its window overlaps `[from, to]`.
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
}

/// Trim to a non-empty owned value, else `None`.
fn clean(v: Option<String>) -> Option<String> {
    v.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

impl TmiListQuery {
    fn into_filters(self) -> tmu_repo::TmiFilters {
        tmu_repo::TmiFilters {
            status: clean(self.status),
            kind: clean(self.kind),
            facility: clean(self.facility),
            from: self.from,
            to: self.to,
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/tmu/tmis",
    tag = "tmu",
    params(
        ("status" = Option<String>, Query, description = "Filter by status (draft|published|expired|cancelled)"),
        ("type" = Option<String>, Query, description = "Filter by structured restriction kind (MIT, MINIT, STOP, …); excludes raw-typed TMIs"),
        ("facility" = Option<String>, Query, description = "Filter to TMIs where this facility is requesting or providing"),
        ("from" = Option<String>, Query, description = "Active-during range start (RFC 3339)"),
        ("to" = Option<String>, Query, description = "Active-during range end (RFC 3339)"),
    ),
    responses((status = 200, body = Vec<TmiBody>), (status = 401))
)]
pub async fn list_tmis(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuTmiRead>,
    Query(query): Query<TmiListQuery>,
) -> Result<Json<Vec<TmiBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(
        tmu_repo::list_tmis(pool, &query.into_filters()).await?,
    ))
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

    payload.requesting = payload.requesting.trim().to_ascii_uppercase();
    payload.providing = payload.providing.trim().to_ascii_uppercase();
    // A structured (form-built) TMI derives its raw line from the fields; a raw TMI uses the text.
    if let Some(s) = &payload.structured {
        if s.element.trim().is_empty() || s.kind.trim().is_empty() {
            return Err(ApiError::BadRequest);
        }
        payload.restriction = crate::tmi::encode(s);
    }
    payload.restriction = payload.restriction.trim().to_string();
    if payload.requesting.is_empty()
        || payload.providing.is_empty()
        || payload.restriction.is_empty()
    {
        return Err(ApiError::BadRequest);
    }

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
    if let Some(requesting) = &payload.requesting {
        payload.requesting = Some(requesting.trim().to_ascii_uppercase());
    }
    if let Some(providing) = &payload.providing {
        payload.providing = Some(providing.trim().to_ascii_uppercase());
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

    // Resolve the target channel before the tx; no config just means "don't post" (skip enqueue).
    // Unscoped: a TMI has requesting/providing ARTCCs but no single "issuing facility", and the TMU
    // channel may be network-wide rather than per-facility (#194).
    let channel = integration_repo::channel_id(pool, TMU_CHANNEL, None).await?;
    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    let tmi = tmu_repo::publish_tmi(&mut tx, &id, &user.id)
        .await?
        .ok_or(ApiError::Conflict)?; // not a draft (or absent)
    if let Some(channel_id) = channel {
        // Enqueued in the same tx as the publish: the advisory can't post without the TMI going live.
        let job = json!({
            "channel_id": channel_id,
            "tmi_id": tmi.id,
            "requesting": tmi.requesting,
            "providing": tmi.providing,
            "restriction": tmi.restriction,
            "start_time": tmi.start_time,
            "stop_time": tmi.stop_time,
        });
        integration_repo::enqueue_job(&mut tx, "tmi_publish", &job, Some("tmi"), Some(&tmi.id))
            .await?;
    }
    tx.commit().await.map_err(|_| ApiError::Internal)?;

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

// --- rate programs ---

/// Uppercase alphanumerics only; keep 3–4 char ICAOs, else reject.
fn normalize_icao(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase();
    (cleaned.len() >= 3 && cleaned.len() <= 4).then_some(cleaned)
}

fn clean_alnum(raw: &str, min: usize, max: usize) -> Option<String> {
    let s: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase();
    (s.len() >= min && s.len() <= max).then_some(s)
}

/// Normalize a program payload the way vatflow's `normRate` does: clamp/validate the
/// airport-wide fields (400 on a bad AAR), drop malformed gates/exclusions, and enforce
/// that miles-in-trail overrides minutes-in-trail. Returns the normalized gate list.
fn normalize_program(payload: &mut UpsertProgramRequest) -> Result<Vec<GateRule>, ApiError> {
    if !(1..=200).contains(&payload.aar) {
        return Err(ApiError::BadRequest);
    }
    payload.trail = payload.trail.clamp(0, 60);
    payload.mit = payload.mit.clamp(0, 300);
    if payload.mit > 0 {
        payload.trail = 0; // MIT overrides minutes-in-trail at the airport level.
    }

    let gates: Vec<GateRule> = payload
        .gates
        .iter()
        .filter_map(|g| {
            let name = clean_alnum(&g.name, 1, 8)?;
            let mit = g.mit.clamp(0, 300);
            let trail = if mit > 0 { 0 } else { g.trail.clamp(0, 60) };
            Some(GateRule { name, trail, mit })
        })
        .take(10)
        .collect();

    payload.exclude_wake = payload
        .exclude_wake
        .iter()
        .map(|w| w.trim().to_ascii_uppercase())
        .filter(|w| matches!(w.as_str(), "L" | "M" | "H" | "J"))
        .collect();
    payload.exclude_types = payload
        .exclude_types
        .iter()
        .filter_map(|t| clean_alnum(t, 2, 4))
        .collect();

    Ok(gates)
}

#[utoipa::path(
    get,
    path = "/api/v1/tmu/programs",
    tag = "tmu",
    responses((status = 200, body = Vec<ProgramBody>), (status = 401))
)]
pub async fn list_programs(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuProgramRead>,
) -> Result<Json<Vec<ProgramBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(tmu_repo::list_programs(pool).await?))
}

#[utoipa::path(
    put,
    path = "/api/v1/tmu/programs/{icao}",
    tag = "tmu",
    params(("icao" = String, Path, description = "Airport ICAO")),
    request_body = UpsertProgramRequest,
    responses((status = 200, body = ProgramBody), (status = 400), (status = 401))
)]
pub async fn upsert_program(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuProgramUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(icao): Path<String>,
    Json(mut payload): Json<UpsertProgramRequest>,
) -> Result<Json<ProgramBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    let gates = normalize_program(&mut payload)?;

    tmu_repo::upsert_program(pool, &icao, &payload, &gates, &user.id).await?;
    let program = tmu_repo::get_program(pool, &icao)
        .await?
        .ok_or(ApiError::Internal)?;
    Ok(Json(program))
}

#[utoipa::path(
    delete,
    path = "/api/v1/tmu/programs/{icao}",
    tag = "tmu",
    params(("icao" = String, Path, description = "Airport ICAO")),
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn delete_program(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuProgramDelete>,
    Path(icao): Path<String>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    if !tmu_repo::delete_program(pool, &icao).await? {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

// --- ground stops ---

/// Normalize the scope: uppercase ARTCC/FIR codes, single-spaced. Empty = field-wide.
fn normalize_scope(raw: Option<&str>) -> String {
    raw.unwrap_or("")
        .split_whitespace()
        .map(|c| c.to_ascii_uppercase())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Normalize an "until" clock time to canonical `HHMM`. Blank -> None (until further
/// notice); anything present but not a valid HHMM -> Err (400).
fn normalize_until(raw: Option<&str>) -> Result<Option<String>, ApiError> {
    let digits: String = raw
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return Ok(None);
    }
    let padded = if digits.len() == 3 {
        format!("0{digits}")
    } else {
        digits
    };
    if padded.len() != 4 {
        return Err(ApiError::BadRequest);
    }
    let hh: u32 = padded[0..2].parse().map_err(|_| ApiError::BadRequest)?;
    let mm: u32 = padded[2..4].parse().map_err(|_| ApiError::BadRequest)?;
    if hh >= 24 || mm >= 60 {
        return Err(ApiError::BadRequest);
    }
    Ok(Some(padded))
}

#[utoipa::path(
    get,
    path = "/api/v1/tmu/ground-stops",
    tag = "tmu",
    responses((status = 200, body = Vec<GroundStopBody>), (status = 401))
)]
pub async fn list_ground_stops(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGroundStopRead>,
) -> Result<Json<Vec<GroundStopBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(tmu_repo::list_ground_stops(pool).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/tmu/ground-stops",
    tag = "tmu",
    request_body = CreateGroundStopRequest,
    responses((status = 200, body = GroundStopBody), (status = 400), (status = 401))
)]
pub async fn create_ground_stop(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGroundStopCreate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Json(mut payload): Json<CreateGroundStopRequest>,
) -> Result<Json<GroundStopBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    payload.airport = payload
        .airport
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase();
    if payload.airport.len() < 3 || payload.airport.len() > 4 {
        return Err(ApiError::BadRequest);
    }
    let scope = normalize_scope(payload.scope.as_deref());
    let until = normalize_until(payload.until.as_deref())?;

    let id =
        tmu_repo::create_ground_stop(pool, &payload, &scope, until.as_deref(), &user.id).await?;
    let gs = tmu_repo::get_ground_stop(pool, &id)
        .await?
        .ok_or(ApiError::Internal)?;
    Ok(Json(gs))
}

#[utoipa::path(
    post,
    path = "/api/v1/tmu/ground-stops/{id}/publish",
    tag = "tmu",
    params(("id" = String, Path, description = "Ground stop id")),
    responses((status = 200, body = GroundStopBody), (status = 401), (status = 409))
)]
pub async fn publish_ground_stop(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGroundStopPublish>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
) -> Result<Json<GroundStopBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if !tmu_repo::publish_ground_stop(pool, &id, &user.id).await? {
        return Err(ApiError::Conflict); // not a draft (or absent)
    }
    let gs = tmu_repo::get_ground_stop(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(gs))
}

#[utoipa::path(
    post,
    path = "/api/v1/tmu/ground-stops/{id}/cancel",
    tag = "tmu",
    params(("id" = String, Path, description = "Ground stop id")),
    responses((status = 200, body = GroundStopBody), (status = 401), (status = 409))
)]
pub async fn cancel_ground_stop(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGroundStopPublish>,
    Path(id): Path<String>,
) -> Result<Json<GroundStopBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if !tmu_repo::cancel_ground_stop(pool, &id).await? {
        return Err(ApiError::Conflict);
    }
    let gs = tmu_repo::get_ground_stop(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(gs))
}

#[utoipa::path(
    delete,
    path = "/api/v1/tmu/ground-stops/{id}",
    tag = "tmu",
    params(("id" = String, Path, description = "Ground stop id")),
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn delete_ground_stop(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGroundStopDelete>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if !tmu_repo::delete_ground_stop(pool, &id).await? {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icao_normalization() {
        assert_eq!(normalize_icao("kjfk").as_deref(), Some("KJFK"));
        assert_eq!(normalize_icao("k jfk!").as_deref(), Some("KJFK"));
        assert_eq!(normalize_icao("zzz").as_deref(), Some("ZZZ"));
        assert_eq!(normalize_icao("xx"), None); // too short
        assert_eq!(normalize_icao("toolong"), None); // too long
    }

    #[test]
    fn clean_alnum_bounds() {
        assert_eq!(clean_alnum("c172", 2, 4).as_deref(), Some("C172"));
        assert_eq!(clean_alnum("pa-28", 2, 4).as_deref(), Some("PA28"));
        assert_eq!(clean_alnum("x", 2, 4), None);
        assert_eq!(clean_alnum("toolong", 2, 4), None);
    }

    #[test]
    fn tmi_list_query_into_filters_trims_and_drops_blanks() {
        let q = TmiListQuery {
            status: Some("  published ".into()),
            kind: Some("MIT".into()),
            facility: Some("   ".into()), // blank → None
            from: None,
            to: None,
        };
        let f = q.into_filters();
        assert_eq!(f.status.as_deref(), Some("published"));
        assert_eq!(f.kind.as_deref(), Some("MIT"));
        assert_eq!(f.facility, None);

        // An empty query yields no constraints.
        let f = TmiListQuery::default().into_filters();
        assert!(
            f.status.is_none()
                && f.kind.is_none()
                && f.facility.is_none()
                && f.from.is_none()
                && f.to.is_none()
        );
    }

    #[test]
    fn scope_normalization() {
        assert_eq!(normalize_scope(Some("ztl  zjx")), "ZTL ZJX");
        assert_eq!(normalize_scope(Some("  zdc ")), "ZDC");
        assert_eq!(normalize_scope(None), "");
    }

    #[test]
    fn until_normalization() {
        assert_eq!(
            normalize_until(Some("200z")).unwrap().as_deref(),
            Some("0200")
        );
        assert_eq!(
            normalize_until(Some("1430")).unwrap().as_deref(),
            Some("1430")
        );
        assert_eq!(normalize_until(Some("")).unwrap(), None);
        assert_eq!(normalize_until(None).unwrap(), None);
        assert!(normalize_until(Some("9999")).is_err()); // hour 99
        assert!(normalize_until(Some("2460")).is_err()); // hour 24
        assert!(normalize_until(Some("1275")).is_err()); // minute 75
    }
}
