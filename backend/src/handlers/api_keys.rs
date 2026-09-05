//! User-owned API keys (personal access tokens). Self-service endpoints let a user with
//! `api_keys.key.create` manage their OWN keys; the plaintext `ois_pat_…` token is shown once, on
//! create/rotate. A key can only be granted permissions its owner currently holds (validated here
//! and re-capped on every request). Admin endpoints (`api_keys.key.read`/`.delete`, or SERVER_ADMIN)
//! oversee all keys. Every lifecycle change is written to the audit log with a before/after snapshot.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::{
    auth::{
        acl::{PermissionPath, fetch_user_access},
        context::CurrentUser,
        permissions::{ApiKeysKeyCreate, ApiKeysKeyDelete, ApiKeysKeyRead},
        require_permission::{Permission, RequirePermission},
    },
    errors::ApiError,
    models::{
        ApiKeyBody, ApiKeyPermissionBody, ApiKeyPermissionInput, ApiKeyTokenBody, AuditLogPage,
        CreateApiKeyRequest, GrantablePermissionBody, RevokeApiKeyRequest,
        SetApiKeyPermissionsRequest,
    },
    repos::{access as access_repo, api_keys as keys_repo, audit as audit_repo},
    state::AppState,
};

const MAX_PERMISSIONS: usize = 200;

/// Mint a token and its public display prefix (`ois_pat_` + 6 hex).
fn generate_token() -> (String, String) {
    let token = format!(
        "ois_pat_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    );
    let prefix: String = token.chars().take(14).collect();
    (token, prefix)
}

/// Normalize the requested grants: trim permission names, upper-case ARTCC ids, drop blanks.
fn to_pairs(permissions: &[ApiKeyPermissionInput]) -> Vec<(String, Option<String>)> {
    permissions
        .iter()
        .map(|p| {
            let name = p.permission.trim().to_string();
            let artcc = p
                .artcc_id
                .as_deref()
                .map(|a| a.trim().to_ascii_uppercase())
                .filter(|a| !a.is_empty());
            (name, artcc)
        })
        .filter(|(name, _)| !name.is_empty())
        .collect()
}

/// A JSON snapshot of a key's granted permissions, for the audit before/after fields.
fn perms_snapshot(perms: &[ApiKeyPermissionBody]) -> Value {
    serde_json::to_value(perms).unwrap_or(Value::Null)
}

/// The signed-in user (keys are user-owned; a request must be a session, not another key).
fn require_user(current_user: &Option<CurrentUser>) -> Result<&CurrentUser, ApiError> {
    current_user.as_ref().ok_or(ApiError::Unauthorized)
}

/// Whether `user_id` holds `path` (used for owner-or-admin checks on the dossier).
async fn user_has(pool: &sqlx::PgPool, user_id: &str, path: PermissionPath) -> bool {
    fetch_user_access(Some(pool), user_id)
        .await
        .map(|(_, perms)| perms.contains(&path))
        .unwrap_or(false)
}

/// Confirm the key exists and is owned by `user_id`; a non-owner gets `NotFound` (existence hidden).
async fn ensure_owner(pool: &sqlx::PgPool, key_id: &str, user_id: &str) -> Result<(), ApiError> {
    match keys_repo::fetch_key_owner(pool, key_id).await? {
        Some(owner) if owner == user_id => Ok(()),
        _ => Err(ApiError::NotFound),
    }
}

/// Best-effort audit of a key lifecycle change, attributed to the acting user.
#[allow(clippy::too_many_arguments)]
async fn audit_key(
    pool: &sqlx::PgPool,
    user: &CurrentUser,
    headers: &HeaderMap,
    action: &str,
    key_id: &str,
    reason: Option<String>,
    before: Option<Value>,
    after: Option<Value>,
) {
    let actor_id = audit_repo::resolve_user_actor_id(pool, &user.id, &user.display_name)
        .await
        .ok()
        .flatten();
    let _ = audit_repo::record_audit(
        pool,
        audit_repo::AuditEntry {
            actor_id,
            action: action.to_string(),
            resource_type: "API_KEY".to_string(),
            resource_id: Some(key_id.to_string()),
            artcc_id: None,
            reason: reason
                .as_deref()
                .map(str::trim)
                .filter(|r| !r.is_empty())
                .map(ToOwned::to_owned),
            before_state: before,
            after_state: after,
            ip_address: audit_repo::client_ip(headers),
        },
    )
    .await;
}

// --- self-service (owner manages their own keys) ---

#[utoipa::path(
    get, path = "/api/v1/api-keys", tag = "api-keys",
    responses((status = 200, body = Vec<ApiKeyBody>), (status = 401))
)]
pub async fn list_my_keys(
    State(state): State<AppState>,
    _permission: RequirePermission<ApiKeysKeyCreate>,
    axum::extract::Extension(current_user): axum::extract::Extension<Option<CurrentUser>>,
) -> Result<Json<Vec<ApiKeyBody>>, ApiError> {
    let user = require_user(&current_user)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(keys_repo::list_keys_for_owner(pool, &user.id).await?))
}

#[utoipa::path(
    get, path = "/api/v1/api-keys/grantable-permissions", tag = "api-keys",
    responses((status = 200, body = Vec<GrantablePermissionBody>), (status = 401))
)]
pub async fn grantable_permissions(
    State(state): State<AppState>,
    _permission: RequirePermission<ApiKeysKeyCreate>,
    axum::extract::Extension(current_user): axum::extract::Extension<Option<CurrentUser>>,
) -> Result<Json<Vec<GrantablePermissionBody>>, ApiError> {
    let user = require_user(&current_user)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    // Everything the owner effectively holds, minus what a key may never hold, with the scope they
    // can delegate for each (national ⇒ any ARTCC; otherwise the specific set).
    let names = access_repo::fetch_user_permission_names(pool, &user.id).await?;
    let mut out = Vec::new();
    for permission in names {
        if keys_repo::is_forbidden_for_key(&permission) {
            continue;
        }
        let (national, artccs) =
            match access_repo::permission_scope(pool, &user.id, &permission).await? {
                access_repo::PermissionScope::National => (true, Vec::new()),
                access_repo::PermissionScope::Facilities(set) => {
                    let mut v: Vec<String> = set.into_iter().collect();
                    v.sort();
                    (false, v)
                }
            };
        out.push(GrantablePermissionBody {
            permission,
            national,
            artccs,
        });
    }
    out.sort_by(|a, b| a.permission.cmp(&b.permission));
    Ok(Json(out))
}

#[utoipa::path(
    post, path = "/api/v1/api-keys", tag = "api-keys",
    request_body = CreateApiKeyRequest,
    responses((status = 200, description = "Created; token shown once", body = ApiKeyTokenBody), (status = 400), (status = 401), (status = 403))
)]
pub async fn create_key(
    State(state): State<AppState>,
    _permission: RequirePermission<ApiKeysKeyCreate>,
    axum::extract::Extension(current_user): axum::extract::Extension<Option<CurrentUser>>,
    headers: HeaderMap,
    Json(payload): Json<CreateApiKeyRequest>,
) -> Result<Json<ApiKeyTokenBody>, ApiError> {
    let user = require_user(&current_user)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    let name = payload.name.trim();
    if name.is_empty() || name.len() > 64 || payload.permissions.len() > MAX_PERMISSIONS {
        return Err(ApiError::BadRequest);
    }
    let description = payload
        .description
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty());
    let pairs = to_pairs(&payload.permissions);

    // No-escalation gate: every grant must be within the owner's live authority.
    keys_repo::validate_subset(pool, &user.id, &pairs).await?;

    let (token, prefix) = generate_token();
    let secret_hash = access_repo::sha256_hex(&token);
    let id = keys_repo::create_api_key(
        pool,
        &user.id,
        name,
        description,
        &prefix,
        &secret_hash,
        payload.expires_at,
        &pairs,
    )
    .await?;

    let key = keys_repo::get_key(pool, &id)
        .await?
        .ok_or(ApiError::Internal)?;
    audit_key(
        pool,
        user,
        &headers,
        "create",
        &id,
        payload.reason,
        None,
        Some(perms_snapshot(&key.permissions)),
    )
    .await;
    Ok(Json(ApiKeyTokenBody { key, token }))
}

#[utoipa::path(
    get, path = "/api/v1/api-keys/{id}", tag = "api-keys",
    params(("id" = String, Path)),
    responses((status = 200, body = ApiKeyBody), (status = 401), (status = 404))
)]
pub async fn get_my_key(
    State(state): State<AppState>,
    _permission: RequirePermission<ApiKeysKeyCreate>,
    axum::extract::Extension(current_user): axum::extract::Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
) -> Result<Json<ApiKeyBody>, ApiError> {
    let user = require_user(&current_user)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    ensure_owner(pool, &id, &user.id).await?;
    keys_repo::get_key(pool, &id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

#[utoipa::path(
    post, path = "/api/v1/api-keys/{id}/rotate", tag = "api-keys",
    params(("id" = String, Path)),
    responses((status = 200, description = "Rotated; new token shown once", body = ApiKeyTokenBody), (status = 401), (status = 404))
)]
pub async fn rotate_key(
    State(state): State<AppState>,
    _permission: RequirePermission<ApiKeysKeyCreate>,
    axum::extract::Extension(current_user): axum::extract::Extension<Option<CurrentUser>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<ApiKeyTokenBody>, ApiError> {
    let user = require_user(&current_user)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    ensure_owner(pool, &id, &user.id).await?;

    let (token, prefix) = generate_token();
    if !keys_repo::rotate_key(pool, &id, &prefix, &access_repo::sha256_hex(&token)).await? {
        return Err(ApiError::NotFound);
    }
    let key = keys_repo::get_key(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    audit_key(pool, user, &headers, "rotate", &id, None, None, None).await;
    Ok(Json(ApiKeyTokenBody { key, token }))
}

#[utoipa::path(
    put, path = "/api/v1/api-keys/{id}/permissions", tag = "api-keys",
    params(("id" = String, Path)), request_body = SetApiKeyPermissionsRequest,
    responses((status = 200, body = ApiKeyBody), (status = 400), (status = 401), (status = 403), (status = 404))
)]
pub async fn set_key_permissions(
    State(state): State<AppState>,
    _permission: RequirePermission<ApiKeysKeyCreate>,
    axum::extract::Extension(current_user): axum::extract::Extension<Option<CurrentUser>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<SetApiKeyPermissionsRequest>,
) -> Result<Json<ApiKeyBody>, ApiError> {
    let user = require_user(&current_user)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    ensure_owner(pool, &id, &user.id).await?;
    if payload.permissions.len() > MAX_PERMISSIONS {
        return Err(ApiError::BadRequest);
    }

    let before = keys_repo::get_key_permissions(pool, &id).await?;
    let pairs = to_pairs(&payload.permissions);
    keys_repo::validate_subset(pool, &user.id, &pairs).await?;
    keys_repo::replace_key_permissions(pool, &id, &pairs).await?;

    let key = keys_repo::get_key(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    audit_key(
        pool,
        user,
        &headers,
        "update",
        &id,
        payload.reason,
        Some(perms_snapshot(&before)),
        Some(perms_snapshot(&key.permissions)),
    )
    .await;
    Ok(Json(key))
}

#[utoipa::path(
    post, path = "/api/v1/api-keys/{id}/disable", tag = "api-keys",
    params(("id" = String, Path)),
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn disable_my_key(
    State(state): State<AppState>,
    _permission: RequirePermission<ApiKeysKeyCreate>,
    axum::extract::Extension(current_user): axum::extract::Extension<Option<CurrentUser>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let user = require_user(&current_user)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    ensure_owner(pool, &id, &user.id).await?;
    keys_repo::disable_key(pool, &id).await?;
    audit_key(pool, user, &headers, "disable", &id, None, None, None).await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete, path = "/api/v1/api-keys/{id}", tag = "api-keys",
    params(("id" = String, Path)),
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn delete_my_key(
    State(state): State<AppState>,
    _permission: RequirePermission<ApiKeysKeyCreate>,
    axum::extract::Extension(current_user): axum::extract::Extension<Option<CurrentUser>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let user = require_user(&current_user)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    ensure_owner(pool, &id, &user.id).await?;
    let before = keys_repo::get_key_permissions(pool, &id).await?;
    if !keys_repo::delete_key(pool, &id).await? {
        return Err(ApiError::NotFound);
    }
    audit_key(
        pool,
        user,
        &headers,
        "delete",
        &id,
        None,
        Some(perms_snapshot(&before)),
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct AuditPageQuery {
    page: Option<i64>,
    page_size: Option<i64>,
}

#[utoipa::path(
    get, path = "/api/v1/api-keys/{id}/audit", tag = "api-keys",
    params(
        ("id" = String, Path),
        ("page" = Option<i64>, Query), ("page_size" = Option<i64>, Query)
    ),
    responses((status = 200, body = AuditLogPage), (status = 401), (status = 404))
)]
pub async fn key_audit(
    State(state): State<AppState>,
    axum::extract::Extension(current_user): axum::extract::Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
    Query(query): Query<AuditPageQuery>,
) -> Result<Json<AuditLogPage>, ApiError> {
    // No static permission gate: this endpoint is authorized manually as "owner OR oversight
    // (api_keys.key.read)" — an oversight admin may lack api_keys.key.create.
    let user = require_user(&current_user)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    // Owner sees their own key's activity; an oversight holder (api_keys.key.read) sees any.
    let owns = matches!(keys_repo::fetch_key_owner(pool, &id).await?, Some(o) if o == user.id);
    if !owns && !user_has(pool, &user.id, ApiKeysKeyRead::path()).await {
        return Err(ApiError::NotFound);
    }

    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(50).clamp(1, 100);

    // No actor row yet ⇒ the key has never acted ⇒ an empty dossier.
    let Some(actor_id) = audit_repo::fetch_api_key_actor_id(pool, &id).await? else {
        return Ok(Json(AuditLogPage {
            items: Vec::new(),
            total: 0,
            page,
            page_size,
        }));
    };
    let filters = audit_repo::AuditLogFilters {
        resource_type: None,
        resource_id: None,
        action: None,
        actor_id: Some(actor_id),
        search: None,
        from: None,
        to: None,
        limit: page_size,
        offset: (page - 1) * page_size,
    };
    let total = audit_repo::count_audit_logs(pool, &filters).await?;
    let items = audit_repo::fetch_audit_logs(pool, &filters).await?;
    Ok(Json(AuditLogPage {
        items,
        total,
        page,
        page_size,
    }))
}

// --- admin oversight (any user's keys) ---

#[derive(Deserialize)]
pub struct AdminKeysQuery {
    owner_cid: Option<i64>,
}

#[utoipa::path(
    get, path = "/api/v1/admin/api-keys", tag = "api-keys",
    params(("owner_cid" = Option<i64>, Query, description = "Filter to one owner's keys")),
    responses((status = 200, body = Vec<ApiKeyBody>), (status = 401))
)]
pub async fn admin_list_keys(
    State(state): State<AppState>,
    _permission: RequirePermission<ApiKeysKeyRead>,
    Query(query): Query<AdminKeysQuery>,
) -> Result<Json<Vec<ApiKeyBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(keys_repo::list_all_keys(pool, query.owner_cid).await?))
}

#[utoipa::path(
    post, path = "/api/v1/admin/api-keys/{id}/disable", tag = "api-keys",
    params(("id" = String, Path)), request_body = RevokeApiKeyRequest,
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn admin_disable_key(
    State(state): State<AppState>,
    _permission: RequirePermission<ApiKeysKeyDelete>,
    axum::extract::Extension(current_user): axum::extract::Extension<Option<CurrentUser>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(payload): Json<RevokeApiKeyRequest>,
) -> Result<StatusCode, ApiError> {
    let user = require_user(&current_user)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if !keys_repo::disable_key(pool, &id).await?
        && keys_repo::fetch_key_owner(pool, &id).await?.is_none()
    {
        return Err(ApiError::NotFound);
    }
    audit_key(
        pool,
        user,
        &headers,
        "disable",
        &id,
        payload.reason,
        None,
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete, path = "/api/v1/admin/api-keys/{id}", tag = "api-keys",
    params(("id" = String, Path)),
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn admin_delete_key(
    State(state): State<AppState>,
    _permission: RequirePermission<ApiKeysKeyDelete>,
    axum::extract::Extension(current_user): axum::extract::Extension<Option<CurrentUser>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let user = require_user(&current_user)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let before = keys_repo::get_key_permissions(pool, &id).await?;
    if !keys_repo::delete_key(pool, &id).await? {
        return Err(ApiError::NotFound);
    }
    audit_key(
        pool,
        user,
        &headers,
        "delete",
        &id,
        None,
        Some(perms_snapshot(&before)),
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}
