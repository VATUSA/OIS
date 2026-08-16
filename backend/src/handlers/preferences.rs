//! Self-scoped per-user preferences (opaque client-owned jsonb), e.g. the dashboard layout.
//! Both routes are gated by `AuthProfileRead` (every signed-in user has it) and operate only on
//! the caller's own row — the user id comes from the session, never the request.

use axum::{
    Extension, Json,
    extract::{Path, State},
};
use serde_json::{Value, json};

use crate::{
    auth::{
        context::CurrentUser, permissions::AuthProfileRead, require_permission::RequirePermission,
    },
    errors::ApiError,
    repos,
    state::AppState,
};

/// Defensive cap on a stored blob — preferences are opaque and client-owned.
const MAX_PREF_BYTES: usize = 256 * 1024;

#[utoipa::path(
    get,
    path = "/api/v1/me/preferences/{namespace}",
    tag = "auth",
    params(("namespace" = String, Path, description = "Preference namespace, e.g. \"dashboard\"")),
    responses((status = 200, body = serde_json::Value), (status = 401))
)]
pub async fn get_preferences(
    State(state): State<AppState>,
    _permission: RequirePermission<AuthProfileRead>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(namespace): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let value = repos::preferences::get_pref(pool, &user.id, &namespace)
        .await?
        .unwrap_or_else(|| json!({}));
    Ok(Json(value))
}

#[utoipa::path(
    put,
    path = "/api/v1/me/preferences/{namespace}",
    tag = "auth",
    params(("namespace" = String, Path, description = "Preference namespace, e.g. \"dashboard\"")),
    request_body = serde_json::Value,
    responses((status = 200, body = serde_json::Value), (status = 400), (status = 401))
)]
pub async fn put_preferences(
    State(state): State<AppState>,
    _permission: RequirePermission<AuthProfileRead>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(namespace): Path<String>,
    Json(value): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let bytes = serde_json::to_vec(&value).map_err(|_| ApiError::Internal)?;
    if bytes.len() > MAX_PREF_BYTES {
        return Err(ApiError::BadRequest);
    }
    repos::preferences::put_pref(pool, &user.id, &namespace, &value).await?;
    Ok(Json(value))
}
