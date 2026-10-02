//! DB-backed access resolution. The pure permission types + tree logic live in
//! `ois-core`; this module adds the Postgres lookups that resolve a user's or
//! service account's effective permissions.

use sqlx::PgPool;

// Re-export the pure core so `require_permission`'s macro and handlers can refer to
// `crate::auth::acl::{PermissionPath, PermissionAction}` as osmium did.
pub use ois_core::catalog::SERVER_ADMIN_ROLE;
pub use ois_core::permissions::{
    PermissionAction, PermissionPath, normalize_permission_tree, permission_tree_from_paths,
};

use serde_json::Value;

use crate::{
    errors::ApiError,
    repos::{access as access_repo, api_keys as api_keys_repo},
};

pub fn is_server_admin(roles: &[String]) -> bool {
    roles.iter().any(|role| role == SERVER_ADMIN_ROLE)
}

/// Builds the nested permission-tree JSON the access editor renders from a flat list
/// of `segments.action` names.
pub fn permission_tree_from_names(names: &[String]) -> Result<Value, ApiError> {
    let paths = access_repo::permission_names_to_permissions(names.to_vec())?;
    Ok(permission_tree_from_paths(&paths))
}

pub async fn fetch_user_access(
    pool: Option<&PgPool>,
    user_id: &str,
) -> Result<(Vec<String>, Vec<PermissionPath>), ApiError> {
    let Some(pool) = pool else {
        return Ok((Vec::new(), Vec::new()));
    };

    let roles = access_repo::fetch_user_role_names(pool, user_id).await?;
    let permission_names = access_repo::fetch_user_permission_names(pool, user_id).await?;
    let permissions = access_repo::permission_names_to_permissions(permission_names)?;

    Ok((roles, permissions))
}

pub async fn fetch_service_account_access(
    pool: Option<&PgPool>,
    service_account_id: &str,
) -> Result<(Vec<String>, Vec<PermissionPath>), ApiError> {
    let Some(pool) = pool else {
        return Ok((Vec::new(), Vec::new()));
    };

    let roles = access_repo::fetch_service_account_role_names(pool, service_account_id).await?;
    let permission_names =
        access_repo::fetch_service_account_permission_names(pool, service_account_id).await?;
    let permissions = access_repo::permission_names_to_permissions(permission_names)?;

    Ok((roles, permissions))
}

/// An API key's *capped* effective permissions: the key's granted set intersected with the owner's
/// current effective access. A permission survives only if the owner still effectively holds it at
/// some scope, it isn't denylisted for keys, and the intersection of the owner's scope with the key's
/// granted scope is non-empty. Keys hold no roles.
///
/// One owner resolution for the whole key (#543). This used to run *two* queries per key permission
/// inside the loop — the effective-name set for the deny semantics and `permission_scope` for the
/// scope — because neither resolver answered both halves. Now one call does, and the per-permission
/// query is gone.
pub async fn fetch_api_key_access(
    pool: Option<&PgPool>,
    api_key: &crate::auth::context::CurrentApiKey,
) -> Result<(Vec<String>, Vec<PermissionPath>), ApiError> {
    let Some(pool) = pool else {
        return Ok((Vec::new(), Vec::new()));
    };

    let owner = access_repo::fetch_effective_permissions(pool, &api_key.owner_user_id).await?;
    let key_names = api_keys_repo::fetch_key_permission_names(pool, &api_key.id).await?;

    let mut effective = Vec::new();
    for name in key_names {
        if api_keys_repo::is_forbidden_for_key(&name) {
            continue;
        }
        // Absent, or present but denied down to nothing, both mean the owner no longer holds it.
        let Some(owner_scope) = owner.get(&name).filter(|scope| !scope.is_empty()) else {
            continue;
        };
        let key_scope = api_keys_repo::key_granted_scope(pool, &api_key.id, &name).await?;
        if !owner_scope.intersect(&key_scope).is_empty() {
            effective.push(name);
        }
    }

    let permissions = access_repo::permission_names_to_permissions(effective)?;
    Ok((Vec::new(), permissions))
}
