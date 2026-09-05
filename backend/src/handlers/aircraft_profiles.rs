//! Aircraft performance profiles (see migration 0059) — the configurable climb / cruise / descent
//! schedules the trajectory model resolves against. National reference data: reads need
//! `flow.aircraft_profiles.read`, writes `flow.aircraft_profiles.update` (no ARTCC scope). Every
//! write reloads the in-memory table cached in `AppState` so it takes effect immediately.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Extension, Path, State},
    http::StatusCode,
};

use crate::{
    auth::{
        context::{CurrentApiKey, CurrentUser},
        permissions::{FlowAircraftProfilesRead, FlowAircraftProfilesUpdate},
        principal::Principal,
        require_permission::RequirePermission,
    },
    errors::ApiError,
    models::{AircraftProfileBody, UpsertAircraftProfileRequest},
    repos::aircraft_profiles as profiles_repo,
    state::AppState,
};

/// Normalize `(kind, key)` from the URL to the stored form, rejecting bad combinations. The global
/// default is addressed as `default/default` and stored with an empty key.
fn normalize(kind: &str, key: &str) -> Result<(String, String), ApiError> {
    match kind {
        "default" => Ok(("default".to_string(), String::new())),
        "wake" => {
            let k = key.to_ascii_uppercase();
            matches!(k.as_str(), "L" | "M" | "H" | "J")
                .then_some(("wake".to_string(), k))
                .ok_or(ApiError::BadRequest)
        }
        "type" => {
            let k = key.trim().to_ascii_uppercase();
            ((2..=4).contains(&k.len()) && k.chars().all(|c| c.is_ascii_alphanumeric()))
                .then_some(("type".to_string(), k))
                .ok_or(ApiError::BadRequest)
        }
        _ => Err(ApiError::BadRequest),
    }
}

/// Sanity-bound the performance numbers so a fat-fingered value can't wreck the ETA model.
fn validate(req: &UpsertAircraftProfileRequest) -> Result<(), ApiError> {
    let ias_ok = |v: f64| (30.0..=400.0).contains(&v);
    let fpm_ok = |v: f64| (100.0..=6000.0).contains(&v);
    let mach_ok = |m: Option<f64>| m.is_none_or(|v| (0.1..=1.0).contains(&v));
    let ok = ias_ok(req.climb_ias_lo)
        && ias_ok(req.climb_ias_hi)
        && ias_ok(req.desc_ias_lo)
        && ias_ok(req.desc_ias_hi)
        && fpm_ok(req.climb_fpm_lo)
        && fpm_ok(req.climb_fpm_hi)
        && fpm_ok(req.desc_fpm)
        && (1000.0..=60000.0).contains(&req.service_ceiling_ft)
        && mach_ok(req.climb_mach)
        && mach_ok(req.cruise_mach)
        && mach_ok(req.desc_mach)
        && req.cruise_tas.is_none_or(|v| (40.0..=700.0).contains(&v))
        && req.name.len() <= 64;
    ok.then_some(()).ok_or(ApiError::BadRequest)
}

/// Reload the DB catalog into the cached table so edits apply to live metering at once.
async fn refresh_cache(state: &AppState, pool: &sqlx::PgPool) -> Result<(), ApiError> {
    let table = profiles_repo::load_all(pool).await?;
    state.aircraft_profiles.store(Arc::new(table));
    Ok(())
}

#[utoipa::path(
    get, path = "/api/v1/flow/aircraft-profiles", tag = "flow",
    responses((status = 200, body = Vec<AircraftProfileBody>), (status = 401))
)]
pub async fn list_profiles(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowAircraftProfilesRead>,
) -> Result<Json<Vec<AircraftProfileBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(profiles_repo::list(pool).await?))
}

#[utoipa::path(
    put, path = "/api/v1/flow/aircraft-profiles/{kind}/{key}", tag = "flow",
    params(("kind" = String, Path), ("key" = String, Path)),
    request_body = UpsertAircraftProfileRequest,
    responses((status = 200, body = AircraftProfileBody), (status = 400), (status = 401))
)]
pub async fn upsert_profile(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowAircraftProfilesUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path((kind, key)): Path<(String, String)>,
    Json(req): Json<UpsertAircraftProfileRequest>,
) -> Result<Json<AircraftProfileBody>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let (kind, key) = normalize(&kind, &key)?;
    validate(&req)?;
    let row = profiles_repo::upsert(pool, &kind, &key, &req, principal.user_id()).await?;
    refresh_cache(&state, pool).await?;
    Ok(Json(row))
}

#[utoipa::path(
    delete, path = "/api/v1/flow/aircraft-profiles/{kind}/{key}", tag = "flow",
    params(("kind" = String, Path), ("key" = String, Path)),
    responses((status = 204), (status = 400), (status = 401), (status = 404))
)]
pub async fn delete_profile(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowAircraftProfilesUpdate>,
    Path((kind, key)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let (kind, key) = normalize(&kind, &key)?;
    if !profiles_repo::delete(pool, &kind, &key).await? {
        return Err(ApiError::NotFound);
    }
    refresh_cache(&state, pool).await?;
    Ok(StatusCode::NO_CONTENT)
}
