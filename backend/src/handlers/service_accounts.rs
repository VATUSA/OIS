//! Service-account management — the Discord bot's (and other machine clients')
//! credentials + roles. The plaintext bearer token is shown once, on create/rotate.

use std::collections::BTreeSet;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use uuid::Uuid;

use crate::{
    auth::{
        acl::SERVER_ADMIN_ROLE,
        permissions::{
            ServiceAccountsCreate, ServiceAccountsDelete, ServiceAccountsRead,
            ServiceAccountsUpdate,
        },
        require_permission::RequirePermission,
    },
    errors::ApiError,
    models::{
        CreateServiceAccountRequest, ServiceAccountBody, ServiceAccountTokenBody,
        SetServiceAccountRolesRequest,
    },
    repos::{access as access_repo, service_accounts as sa_repo},
    state::AppState,
};

fn generate_token() -> String {
    format!(
        "ois_sa_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    )
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/service-accounts",
    tag = "service-accounts",
    responses((status = 200, body = Vec<ServiceAccountBody>), (status = 401))
)]
pub async fn list_service_accounts(
    State(state): State<AppState>,
    _permission: RequirePermission<ServiceAccountsRead>,
) -> Result<Json<Vec<ServiceAccountBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(sa_repo::list_service_accounts(pool).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/admin/service-accounts",
    tag = "service-accounts",
    request_body = CreateServiceAccountRequest,
    responses((status = 200, description = "Created; token shown once", body = ServiceAccountTokenBody), (status = 400), (status = 401))
)]
pub async fn create_service_account(
    State(state): State<AppState>,
    _permission: RequirePermission<ServiceAccountsCreate>,
    Json(payload): Json<CreateServiceAccountRequest>,
) -> Result<Json<ServiceAccountTokenBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let name = payload.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest);
    }
    let description = payload
        .description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let token = generate_token();
    let secret_hash = access_repo::sha256_hex(&token);
    let key = format!("sa_{}", Uuid::new_v4().simple());

    let id = sa_repo::create_service_account(pool, &key, name, description, &secret_hash).await?;
    let account = sa_repo::get_service_account(pool, &id)
        .await?
        .ok_or(ApiError::Internal)?;
    Ok(Json(ServiceAccountTokenBody { account, token }))
}

#[utoipa::path(
    post,
    path = "/api/v1/admin/service-accounts/{id}/rotate",
    tag = "service-accounts",
    params(("id" = String, Path, description = "Service account id")),
    responses((status = 200, description = "Rotated; new token shown once", body = ServiceAccountTokenBody), (status = 401), (status = 404))
)]
pub async fn rotate_service_account(
    State(state): State<AppState>,
    _permission: RequirePermission<ServiceAccountsUpdate>,
    Path(id): Path<String>,
) -> Result<Json<ServiceAccountTokenBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let token = generate_token();
    sa_repo::rotate_credential(pool, &id, &access_repo::sha256_hex(&token)).await?;
    let account = sa_repo::get_service_account(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(ServiceAccountTokenBody { account, token }))
}

#[utoipa::path(
    post,
    path = "/api/v1/admin/service-accounts/{id}/disable",
    tag = "service-accounts",
    params(("id" = String, Path, description = "Service account id")),
    responses((status = 204, description = "Disabled + credentials revoked"), (status = 401), (status = 404))
)]
pub async fn disable_service_account(
    State(state): State<AppState>,
    _permission: RequirePermission<ServiceAccountsDelete>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    sa_repo::disable_service_account(pool, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    put,
    path = "/api/v1/admin/service-accounts/{id}/roles",
    tag = "service-accounts",
    params(("id" = String, Path, description = "Service account id")),
    request_body = SetServiceAccountRolesRequest,
    responses((status = 200, body = ServiceAccountBody), (status = 400), (status = 401), (status = 404))
)]
pub async fn set_service_account_roles(
    State(state): State<AppState>,
    _permission: RequirePermission<ServiceAccountsUpdate>,
    Path(id): Path<String>,
    Json(payload): Json<SetServiceAccountRolesRequest>,
) -> Result<Json<ServiceAccountBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    // Roles must exist and never include SERVER_ADMIN (env-bootstrapped only).
    let known: BTreeSet<String> = access_repo::fetch_role_names(pool)
        .await?
        .into_iter()
        .collect();
    for role_name in &payload.role_names {
        if role_name == SERVER_ADMIN_ROLE || !known.contains(role_name) {
            return Err(ApiError::BadRequest);
        }
    }

    sa_repo::set_roles(pool, &id, &payload.role_names).await?;
    let account = sa_repo::get_service_account(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(account))
}
