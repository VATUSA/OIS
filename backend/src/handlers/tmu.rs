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
            TmuAdvCreate, TmuAdvPublish, TmuAdvRead, TmuAdvUpdate, TmuGroundStopCreate,
            TmuGroundStopDelete, TmuGroundStopPublish, TmuGroundStopRead, TmuProgramDelete,
            TmuProgramRead, TmuProgramUpdate, TmuTmiCreate, TmuTmiDelete, TmuTmiPublish,
            TmuTmiRead, TmuTmiUpdate,
        },
        require_permission::RequirePermission,
    },
    errors::ApiError,
    handlers::restriction_artcc,
    models::{
        AdvisoryBody, CreateAdvisoryRequest, CreateGroundStopRequest, CreateTmiRequest, GateRule,
        GroundStopBody, ProgramBody, TmiBody, UpdateAdvisoryRequest, UpdateTmiRequest,
        UpsertProgramRequest,
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
    let mut tmis = tmu_repo::list_tmis(pool, &query.into_filters()).await?;
    restriction_artcc::stamp_tmis(&*state.facilities.read().await, &mut tmis);
    Ok(Json(tmis))
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
    let mut tmi = tmu_repo::get_tmi(pool, &id)
        .await?
        .ok_or(ApiError::Internal)?;
    restriction_artcc::stamp_tmi(&*state.facilities.read().await, &mut tmi);
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
    let mut tmi = tmu_repo::get_tmi(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    restriction_artcc::stamp_tmi(&*state.facilities.read().await, &mut tmi);
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
    let mut tmi = tmu_repo::publish_tmi(&mut tx, &id, &user.id)
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

    restriction_artcc::stamp_tmi(&*state.facilities.read().await, &mut tmi);
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
    let mut tmi = tmu_repo::get_tmi(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    restriction_artcc::stamp_tmi(&*state.facilities.read().await, &mut tmi);
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
    let mut programs = tmu_repo::list_programs(pool).await?;
    restriction_artcc::stamp_programs(&*state.facilities.read().await, &mut programs);
    Ok(Json(programs))
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
    let mut program = tmu_repo::get_program(pool, &icao)
        .await?
        .ok_or(ApiError::Internal)?;
    restriction_artcc::stamp_program(&*state.facilities.read().await, &mut program);
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
    let mut stops = tmu_repo::list_ground_stops(pool).await?;
    restriction_artcc::stamp_ground_stops(&*state.facilities.read().await, &mut stops);
    Ok(Json(stops))
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
    let mut gs = tmu_repo::get_ground_stop(pool, &id)
        .await?
        .ok_or(ApiError::Internal)?;
    restriction_artcc::stamp_ground_stop(&*state.facilities.read().await, &mut gs);
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
    let mut gs = tmu_repo::get_ground_stop(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    restriction_artcc::stamp_ground_stop(&*state.facilities.read().await, &mut gs);
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
    let mut gs = tmu_repo::get_ground_stop(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    restriction_artcc::stamp_ground_stop(&*state.facilities.read().await, &mut gs);
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

    use std::collections::HashMap;

    use sqlx::PgPool;

    use crate::scope_test_support::{grant, seed_user, send, session_cookie, test_state};

    fn new_advisory() -> serde_json::Value {
        serde_json::json!({"facility": "DCC", "kind": "reroute", "body": "vATCSCC ADVZY"})
    }

    async fn seed_draft(pool: &PgPool) -> String {
        let author = seed_user(pool).await;
        crate::repos::tmu::create_advisory(
            pool,
            &CreateAdvisoryRequest {
                facility: "DCC".into(),
                kind: "reroute".into(),
                body: "vATCSCC ADVZY".into(),
                structured: None,
                decoded: None,
            },
            &author,
        )
        .await
        .unwrap()
    }

    /// A signed-in caller holding no permissions, and the state to send through.
    ///
    /// Deliberately two steps rather than one refused-then-granted helper: the interesting assertion
    /// is that the *refused* call changed nothing, and that has to be checked before the granted call
    /// runs — a helper that did both first would let the granted call mask it (which it did, on the
    /// first run of these tests).
    async fn caller(pool: &PgPool) -> (crate::state::AppState, String, String) {
        let state = test_state(pool.clone(), HashMap::new());
        let user = seed_user(pool).await;
        let cookie = session_cookie(pool, &user).await;
        (state, user, cookie)
    }

    async fn status_of(pool: &PgPool, id: &str) -> Option<String> {
        crate::repos::tmu::get_advisory(pool, id)
            .await
            .unwrap()
            .map(|a| a.status)
    }

    /// AC 1's third verb. `cancel_advisory` shares `TmuAdvPublish` with publish, so publish's test
    /// covered the permission but not *this route's* use of it: dropping the extractor here left 21
    /// passed / 0 failed (#457 review).
    #[sqlx::test]
    async fn cancelling_an_advisory_requires_the_publish_permission(pool: PgPool) {
        let id = seed_draft(&pool).await;
        let (state, user, cookie) = caller(&pool).await;
        let uri = format!("/api/v1/tmu/advisories/{id}/cancel");

        let refused = send(&state, http::Method::POST, &uri, &cookie, None).await;
        assert_eq!(refused, http::StatusCode::UNAUTHORIZED);
        assert_eq!(
            status_of(&pool, &id).await.as_deref(),
            Some("draft"),
            "a refused cancel must not have changed the status"
        );

        grant(&pool, &user, "tmu.adv.publish", None).await;
        let allowed = send(&state, http::Method::POST, &uri, &cookie, None).await;
        assert_eq!(allowed, http::StatusCode::OK);
        assert_eq!(status_of(&pool, &id).await.as_deref(), Some("cancelled"));
    }

    /// The remaining two gated routes, for the same reason: each one's extractor should be removable
    /// only at the cost of a red test (#457 review).
    #[sqlx::test]
    async fn editing_an_advisory_requires_the_update_permission(pool: PgPool) {
        let id = seed_draft(&pool).await;
        let (state, user, cookie) = caller(&pool).await;
        let uri = format!("/api/v1/tmu/advisories/{id}");
        let edit = || Some(serde_json::json!({"body": "vATCSCC ADVZY 001 amended"}));
        let body_now = async || {
            crate::repos::tmu::get_advisory(&pool, &id)
                .await
                .unwrap()
                .unwrap()
                .body
        };

        let refused = send(&state, http::Method::PATCH, &uri, &cookie, edit()).await;
        assert_eq!(refused, http::StatusCode::UNAUTHORIZED);
        assert_eq!(
            body_now().await,
            "vATCSCC ADVZY",
            "a refused edit must not have rewritten the document"
        );

        grant(&pool, &user, "tmu.adv.update", None).await;
        let allowed = send(&state, http::Method::PATCH, &uri, &cookie, edit()).await;
        assert_eq!(allowed, http::StatusCode::OK);
        assert_eq!(body_now().await, "vATCSCC ADVZY 001 amended");
    }

    #[sqlx::test]
    async fn abandoning_an_advisory_requires_the_update_permission(pool: PgPool) {
        let id = seed_draft(&pool).await;
        let (state, user, cookie) = caller(&pool).await;
        let uri = format!("/api/v1/tmu/advisories/{id}");

        let refused = send(&state, http::Method::DELETE, &uri, &cookie, None).await;
        assert_eq!(refused, http::StatusCode::UNAUTHORIZED);
        assert!(
            status_of(&pool, &id).await.is_some(),
            "a refused delete must not have removed it"
        );

        grant(&pool, &user, "tmu.adv.update", None).await;
        let allowed = send(&state, http::Method::DELETE, &uri, &cookie, None).await;
        assert_eq!(allowed, http::StatusCode::NO_CONTENT);
        assert!(status_of(&pool, &id).await.is_none());
    }

    /// #457 AC1. `tmu.adv.*` were seeded in 0008_tmu.sql and dead ever since, because nothing could
    /// gate on them. These go through the real router so `RequirePermission` is on the tested path —
    /// it holds a private field, so a handler cannot be called directly to check its gate.
    ///
    /// A missing permission is **401** here, not 403: `ensure_permission` answers `Unauthorized` for
    /// every absent permission, and 403 is reserved for a wrong facility scope.
    #[sqlx::test]
    async fn creating_an_advisory_requires_the_create_permission(pool: PgPool) {
        let state = test_state(pool.clone(), HashMap::new());
        let user = seed_user(&pool).await;
        let cookie = session_cookie(&pool, &user).await;

        let refused = send(
            &state,
            http::Method::POST,
            "/api/v1/tmu/advisories",
            &cookie,
            Some(new_advisory()),
        )
        .await;
        assert_eq!(refused, http::StatusCode::UNAUTHORIZED);
        assert!(
            crate::repos::tmu::list_advisories(&pool)
                .await
                .unwrap()
                .is_empty(),
            "a refused create must not have written anything"
        );

        grant(&pool, &user, "tmu.adv.create", None).await;
        let allowed = send(
            &state,
            http::Method::POST,
            "/api/v1/tmu/advisories",
            &cookie,
            Some(new_advisory()),
        )
        .await;
        assert_eq!(allowed, http::StatusCode::OK);
        assert_eq!(
            crate::repos::tmu::list_advisories(&pool)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    /// Publishing is a separate permission from creating: drafting a document and issuing it are
    /// different levels of trust, which is why 0008_tmu.sql seeded them separately.
    #[sqlx::test]
    async fn publishing_an_advisory_requires_the_publish_permission(pool: PgPool) {
        let state = test_state(pool.clone(), HashMap::new());
        let id = seed_draft(&pool).await;

        let user = seed_user(&pool).await;
        let cookie = session_cookie(&pool, &user).await;
        let uri = format!("/api/v1/tmu/advisories/{id}/publish");

        // Holding create is not holding publish.
        grant(&pool, &user, "tmu.adv.create", None).await;
        assert_eq!(
            send(&state, http::Method::POST, &uri, &cookie, None).await,
            http::StatusCode::UNAUTHORIZED
        );
        let still_draft = crate::repos::tmu::get_advisory(&pool, &id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(still_draft.status, "draft");

        grant(&pool, &user, "tmu.adv.publish", None).await;
        assert_eq!(
            send(&state, http::Method::POST, &uri, &cookie, None).await,
            http::StatusCode::OK
        );
        let published = crate::repos::tmu::get_advisory(&pool, &id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(published.status, "published");
        assert!(published.published_at.is_some());
    }

    /// Reading is gated too — advisories are not public until #459 posts them.
    #[sqlx::test]
    async fn listing_advisories_requires_the_read_permission(pool: PgPool) {
        let state = test_state(pool.clone(), HashMap::new());
        let user = seed_user(&pool).await;
        let cookie = session_cookie(&pool, &user).await;

        assert_eq!(
            send(
                &state,
                http::Method::GET,
                "/api/v1/tmu/advisories",
                &cookie,
                None
            )
            .await,
            http::StatusCode::UNAUTHORIZED
        );

        grant(&pool, &user, "tmu.adv.read", None).await;
        assert_eq!(
            send(
                &state,
                http::Method::GET,
                "/api/v1/tmu/advisories",
                &cookie,
                None
            )
            .await,
            http::StatusCode::OK
        );
    }

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

// --- advisories (ADVZY documents, #457) ---

#[utoipa::path(
    get, path = "/api/v1/tmu/advisories", tag = "tmu",
    responses((status = 200, body = Vec<AdvisoryBody>), (status = 401))
)]
pub async fn list_advisories(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuAdvRead>,
) -> Result<Json<Vec<AdvisoryBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(tmu_repo::list_advisories(pool).await?))
}

#[utoipa::path(
    get, path = "/api/v1/tmu/advisories/{id}", tag = "tmu",
    params(("id" = String, Path)),
    responses((status = 200, body = AdvisoryBody), (status = 401), (status = 404))
)]
pub async fn get_advisory(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuAdvRead>,
    Path(id): Path<String>,
) -> Result<Json<AdvisoryBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    tmu_repo::get_advisory(pool, &id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

/// Creates a draft, allocating its advisory number immediately so the author can see what they will
/// issue under (#457).
#[utoipa::path(
    post, path = "/api/v1/tmu/advisories", tag = "tmu",
    request_body = CreateAdvisoryRequest,
    responses((status = 200, body = AdvisoryBody), (status = 400), (status = 401))
)]
pub async fn create_advisory(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuAdvCreate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Json(payload): Json<CreateAdvisoryRequest>,
) -> Result<Json<AdvisoryBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if payload.facility.trim().is_empty()
        || payload.kind.trim().is_empty()
        || payload.body.trim().is_empty()
    {
        return Err(ApiError::BadRequest);
    }
    let id = tmu_repo::create_advisory(pool, &payload, &user.id).await?;
    tmu_repo::get_advisory(pool, &id)
        .await?
        .map(Json)
        .ok_or(ApiError::Internal)
}

#[utoipa::path(
    patch, path = "/api/v1/tmu/advisories/{id}", tag = "tmu",
    params(("id" = String, Path)), request_body = UpdateAdvisoryRequest,
    responses((status = 200, body = AdvisoryBody), (status = 401), (status = 404))
)]
pub async fn update_advisory(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuAdvUpdate>,
    Path(id): Path<String>,
    Json(payload): Json<UpdateAdvisoryRequest>,
) -> Result<Json<AdvisoryBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if !tmu_repo::update_advisory(pool, &id, &payload).await? {
        return Err(ApiError::NotFound);
    }
    tmu_repo::get_advisory(pool, &id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

#[utoipa::path(
    post, path = "/api/v1/tmu/advisories/{id}/publish", tag = "tmu",
    params(("id" = String, Path)),
    responses((status = 200, body = AdvisoryBody), (status = 401), (status = 409))
)]
pub async fn publish_advisory(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuAdvPublish>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
) -> Result<Json<AdvisoryBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if !tmu_repo::publish_advisory(pool, &id, &user.id).await? {
        return Err(ApiError::Conflict);
    }
    tmu_repo::get_advisory(pool, &id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

#[utoipa::path(
    post, path = "/api/v1/tmu/advisories/{id}/cancel", tag = "tmu",
    params(("id" = String, Path)),
    responses((status = 200, body = AdvisoryBody), (status = 401), (status = 409))
)]
pub async fn cancel_advisory(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuAdvPublish>,
    Path(id): Path<String>,
) -> Result<Json<AdvisoryBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if !tmu_repo::cancel_advisory(pool, &id).await? {
        return Err(ApiError::Conflict);
    }
    tmu_repo::get_advisory(pool, &id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

/// Abandons a draft. `409` when it is not a draft — a published advisory is a document that went out,
/// and a cancelled one is a record of that; neither is deleted. Any draft can be abandoned, whether or
/// not its number is the top of the sequence; abandoning an older one simply leaves a gap (see
/// `repos::tmu::delete_advisory`).
#[utoipa::path(
    delete, path = "/api/v1/tmu/advisories/{id}", tag = "tmu",
    params(("id" = String, Path)),
    responses((status = 204), (status = 401), (status = 409))
)]
pub async fn delete_advisory(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuAdvUpdate>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if tmu_repo::delete_advisory(pool, &id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::Conflict)
    }
}
