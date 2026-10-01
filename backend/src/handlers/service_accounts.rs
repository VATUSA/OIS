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

/// The first requested role that may not be assigned, if any. Extracted so the rejection is
/// testable without an HTTP harness — an empty request is valid and clears the account's roles.
fn first_unassignable<'a>(
    requested: &'a [String],
    assignable: &BTreeSet<String>,
) -> Option<&'a str> {
    requested
        .iter()
        .map(String::as_str)
        .find(|role| !assignable.contains(*role))
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/service-accounts/roles",
    tag = "service-accounts",
    responses(
        (status = 200, description = "Role names a service account may hold", body = Vec<String>),
        (status = 401)
    )
)]
/// The roles assignable to a service account — what the admin UI's picker renders. Gated on
/// Update because holding it is what lets you act on the list.
pub async fn list_service_account_roles(
    State(state): State<AppState>,
    _permission: RequirePermission<ServiceAccountsUpdate>,
) -> Result<Json<Vec<String>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(
        access_repo::fetch_service_account_assignable_roles(pool).await?,
    ))
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

    // Validated against the same list `list_service_account_roles` offers the picker, so the
    // UI can never present a role this rejects. SERVER_ADMIN is already filtered out of it.
    let assignable: BTreeSet<String> = access_repo::fetch_service_account_assignable_roles(pool)
        .await?
        .into_iter()
        .collect();
    if first_unassignable(&payload.role_names, &assignable).is_some() {
        return Err(ApiError::BadRequest);
    }

    sa_repo::set_roles(pool, &id, &payload.role_names).await?;
    let account = sa_repo::get_service_account(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(account))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::first_unassignable;

    fn assignable(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    fn requested(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn a_machine_role_the_picker_offers_is_accepted() {
        let ok = assignable(&["BOT", "SERVICE_APP", "NTMO"]);
        assert_eq!(first_unassignable(&requested(&["BOT"]), &ok), None);
        assert_eq!(first_unassignable(&requested(&["BOT", "NTMO"]), &ok), None);
    }

    /// SERVER_ADMIN is env-bootstrapped only, so it is filtered out of the assignable list and must
    /// therefore be rejected here even though it is a perfectly real role.
    #[test]
    fn server_admin_is_rejected_because_it_is_never_assignable() {
        let ok = assignable(&["BOT", "SERVICE_APP"]);
        assert_eq!(
            first_unassignable(&requested(&["SERVER_ADMIN"]), &ok),
            Some("SERVER_ADMIN")
        );
        // Rejected even when smuggled in alongside a role that is allowed.
        assert_eq!(
            first_unassignable(&requested(&["BOT", "SERVER_ADMIN"]), &ok),
            Some("SERVER_ADMIN")
        );
    }

    #[test]
    fn an_unknown_role_is_rejected() {
        let ok = assignable(&["BOT"]);
        assert_eq!(
            first_unassignable(&requested(&["NOT_A_ROLE"]), &ok),
            Some("NOT_A_ROLE")
        );
    }

    /// Clearing an account's roles is a legitimate request, not an empty-input error.
    #[test]
    fn an_empty_request_clears_roles_rather_than_failing() {
        assert_eq!(first_unassignable(&[], &assignable(&["BOT"])), None);
    }
}
