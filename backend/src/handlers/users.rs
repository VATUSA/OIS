//! User directory search.

use axum::{
    Json,
    extract::{Query, State},
};
use serde::Deserialize;

use crate::{
    auth::{permissions::UsersDirectoryRead, require_permission::RequirePermission},
    errors::ApiError,
    models::UserSummary,
    repos::users as user_repo,
    state::AppState,
};

#[derive(Deserialize)]
pub struct UserSearchQuery {
    q: Option<String>,
    limit: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/v1/users",
    tag = "users",
    params(
        ("q" = Option<String>, Query, description = "Name substring or CID prefix"),
        ("limit" = Option<i64>, Query, description = "Max results (default 20, max 50)")
    ),
    responses((status = 200, body = Vec<UserSummary>), (status = 401))
)]
pub async fn search_users(
    State(state): State<AppState>,
    _permission: RequirePermission<UsersDirectoryRead>,
    Query(query): Query<UserSearchQuery>,
) -> Result<Json<Vec<UserSummary>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let q = query.q.unwrap_or_default();
    let q = q.trim();
    if q.is_empty() {
        return Ok(Json(Vec::new()));
    }
    let limit = query.limit.unwrap_or(20).clamp(1, 50);
    Ok(Json(user_repo::search_users(pool, q, limit).await?))
}
