//! Service-account management — the Discord bot's (and other machine clients')
//! credentials, roles and direct grants. The plaintext bearer token is shown once, on create/rotate.
//!
//! A service account has no owner to cap it, so every grant is capped by the admin making it (#584):
//! roles and permissions alike must be within that admin's own live authority, checked on each write.

use std::collections::BTreeSet;

use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

use crate::{
    auth::{
        context::CurrentUser,
        permissions::{
            ServiceAccountsCreate, ServiceAccountsDelete, ServiceAccountsRead,
            ServiceAccountsUpdate,
        },
        require_permission::RequirePermission,
    },
    errors::ApiError,
    handlers::api_keys::{MAX_PERMISSIONS, to_pairs},
    models::{
        CreateServiceAccountRequest, GrantablePermissionBody, RotateServiceAccountRequest,
        ServiceAccountBody, ServiceAccountTokenBody, SetServiceAccountPermissionsRequest,
        SetServiceAccountRolesRequest,
    },
    repos::{access as access_repo, api_keys as keys_repo, service_accounts as sa_repo},
    state::AppState,
};

/// A credential's lifetime when the admin doesn't choose one, and the longest they may choose.
const DEFAULT_EXPIRY_DAYS: u32 = 90;
const MAX_EXPIRY_DAYS: u32 = 365;

/// When a credential issued now with `requested` days of life expires. Zero or past the maximum is
/// a 400 rather than a silent clamp, so the admin never gets a lifetime they didn't ask for.
fn expiry_from(requested: Option<u32>, now: DateTime<Utc>) -> Result<DateTime<Utc>, ApiError> {
    let days = requested.unwrap_or(DEFAULT_EXPIRY_DAYS);
    if days == 0 || days > MAX_EXPIRY_DAYS {
        return Err(ApiError::BadRequest);
    }
    Ok(now + Duration::days(i64::from(days)))
}

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
    let expires_at = expiry_from(payload.expires_in_days, Utc::now())?;

    let token = generate_token();
    let secret_hash = access_repo::sha256_hex(&token);
    let key = format!("sa_{}", Uuid::new_v4().simple());

    let id =
        sa_repo::create_service_account(pool, &key, name, description, &secret_hash, expires_at)
            .await?;
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
    request_body(content = Option<RotateServiceAccountRequest>, description = "Optional lifetime; default 90 days"),
    responses((status = 200, description = "Rotated; new token shown once", body = ServiceAccountTokenBody), (status = 400), (status = 401), (status = 403), (status = 404))
)]
/// Revoke the live token and issue a new one. Whoever rotates *receives* the token, and with it the
/// account's authority — so, like a grant, it is capped: the admin must hold everything the account
/// holds, at its scope (#584). Otherwise `service_accounts.update` alone would be a way to take BOT.
pub async fn rotate_service_account(
    State(state): State<AppState>,
    _permission: RequirePermission<ServiceAccountsUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
    payload: Option<Json<RotateServiceAccountRequest>>,
) -> Result<Json<ServiceAccountTokenBody>, ApiError> {
    let admin = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let Json(payload) = payload.unwrap_or_default();
    let expires_at = expiry_from(payload.expires_in_days, Utc::now())?;

    let held = access_repo::fetch_service_account_grants(pool, &id).await?;
    keys_repo::validate_grants(
        pool,
        &admin.id,
        &held,
        keys_repo::is_forbidden_for_service_account,
    )
    .await?;

    let token = generate_token();
    sa_repo::rotate_credential(pool, &id, &access_repo::sha256_hex(&token), expires_at).await?;
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
    responses((status = 200, body = ServiceAccountBody), (status = 400), (status = 401), (status = 403), (status = 404))
)]
pub async fn set_service_account_roles(
    State(state): State<AppState>,
    _permission: RequirePermission<ServiceAccountsUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
    Json(payload): Json<SetServiceAccountRolesRequest>,
) -> Result<Json<ServiceAccountBody>, ApiError> {
    let admin = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
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

    // No escalation through a role: a role is granted nationally, so the admin must hold every
    // permission in it nationally. Without this, roles would bypass the per-permission cap below.
    let role_grants: Vec<(String, Option<String>)> =
        access_repo::fetch_role_permission_names(pool, &payload.role_names)
            .await?
            .into_iter()
            .map(|name| (name, None))
            .collect();
    keys_repo::validate_grants(
        pool,
        &admin.id,
        &role_grants,
        keys_repo::is_forbidden_for_service_account,
    )
    .await?;

    sa_repo::set_roles(pool, &id, &payload.role_names).await?;
    let account = sa_repo::get_service_account(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(account))
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/service-accounts/grantable-permissions",
    tag = "service-accounts",
    responses(
        (status = 200, description = "What the caller may grant a service account, with the scope", body = Vec<GrantablePermissionBody>),
        (status = 401)
    )
)]
/// The permission picker's source: what the calling admin holds, minus what a service account may
/// never hold — exactly what `set_service_account_permissions` will accept from them.
pub async fn grantable_service_account_permissions(
    State(state): State<AppState>,
    _permission: RequirePermission<ServiceAccountsUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
) -> Result<Json<Vec<GrantablePermissionBody>>, ApiError> {
    let admin = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(
        keys_repo::grantable_for(pool, &admin.id, keys_repo::is_forbidden_for_service_account)
            .await?,
    ))
}

#[utoipa::path(
    put,
    path = "/api/v1/admin/service-accounts/{id}/permissions",
    tag = "service-accounts",
    params(("id" = String, Path, description = "Service account id")),
    request_body = SetServiceAccountPermissionsRequest,
    responses((status = 200, body = ServiceAccountBody), (status = 400), (status = 401), (status = 403), (status = 404))
)]
/// Replace an account's direct `(permission, ARTCC)` grants (#584). Each must be within the calling
/// admin's own live authority (403 otherwise), and none may let a machine mint credentials (400).
pub async fn set_service_account_permissions(
    State(state): State<AppState>,
    _permission: RequirePermission<ServiceAccountsUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
    Json(payload): Json<SetServiceAccountPermissionsRequest>,
) -> Result<Json<ServiceAccountBody>, ApiError> {
    let admin = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if payload.permissions.len() > MAX_PERMISSIONS {
        return Err(ApiError::BadRequest);
    }
    let grants = to_pairs(&payload.permissions);
    keys_repo::validate_grants(
        pool,
        &admin.id,
        &grants,
        keys_repo::is_forbidden_for_service_account,
    )
    .await?;

    sa_repo::set_permissions(pool, &id, &grants).await?;
    let account = sa_repo::get_service_account(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(account))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use chrono::{Duration, TimeZone, Utc};

    use super::{expiry_from, first_unassignable};
    use crate::errors::ApiError;

    #[test]
    fn expiry_defaults_to_90_days_and_caps_at_365() {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        assert_eq!(expiry_from(None, now).unwrap(), now + Duration::days(90));
        assert_eq!(
            expiry_from(Some(365), now).unwrap(),
            now + Duration::days(365)
        );
        assert!(matches!(
            expiry_from(Some(366), now),
            Err(ApiError::BadRequest)
        ));
        assert!(matches!(
            expiry_from(Some(0), now),
            Err(ApiError::BadRequest)
        ));
    }

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
