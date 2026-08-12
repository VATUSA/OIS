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

use crate::{errors::ApiError, repos::access as access_repo};

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
