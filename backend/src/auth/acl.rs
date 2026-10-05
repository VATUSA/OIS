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

use std::collections::BTreeMap;

use crate::{
    errors::ApiError,
    models::{ScopeAccess, UserAccessBody},
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

/// Groups direct grants + role assignments into per-scope `ScopeAccess` (national first).
///
/// The snapshot every `USER_ACCESS` audit entry records either side of a change — the user editor,
/// the group side (#546 AC6), and VATUSA sync (#548) — so it lives here rather than in a handler,
/// where the sync's repo layer could not share it.
pub fn user_access_body(
    user_id: &str,
    cid: i64,
    grants: Vec<(Option<String>, String)>,
    roles: Vec<(Option<String>, String)>,
) -> Result<UserAccessBody, ApiError> {
    let national_roles: Vec<String> = roles
        .iter()
        .filter(|(artcc, _)| artcc.is_none())
        .map(|(_, role)| role.clone())
        .collect();
    let server_admin = is_server_admin(&national_roles);

    let mut map: BTreeMap<Option<String>, (Vec<String>, Vec<String>)> = BTreeMap::new();
    map.entry(None).or_default(); // national scope always present
    for (artcc, role) in roles {
        map.entry(artcc).or_default().0.push(role);
    }
    for (artcc, permission) in grants {
        map.entry(artcc).or_default().1.push(permission);
    }

    let mut scopes = Vec::with_capacity(map.len());
    for (artcc_id, (role_names, perm_names)) in map {
        scopes.push(ScopeAccess {
            artcc_id,
            role_names,
            permissions: permission_tree_from_names(&perm_names)?,
        });
    }

    Ok(UserAccessBody {
        id: user_id.to_string(),
        cid,
        server_admin,
        scopes,
    })
}

/// A server admin holds the whole catalogue nationally, through the effective-permissions view rather
/// than stored grants — so a snapshot shows it explicitly, or it would read as holding nothing.
pub fn apply_server_admin_catalog(
    body: &mut UserAccessBody,
    catalog: &[String],
) -> Result<(), ApiError> {
    let tree = permission_tree_from_names(catalog)?;
    if let Some(national) = body.scopes.iter_mut().find(|s| s.artcc_id.is_none()) {
        national.permissions = tree;
    }
    Ok(())
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
