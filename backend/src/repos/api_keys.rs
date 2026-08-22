//! User-owned API keys (personal access tokens). This module owns the key's *authority* side:
//! the granted (permission, scope) subset, the denylist, and the "requested set ⊆ owner's live
//! access" validation used when a key is created or edited. The capped effective set (key grants ∩
//! owner's current access) is assembled in `auth::acl::fetch_api_key_access`.

use std::collections::HashSet;

use sqlx::PgPool;

use crate::{
    errors::ApiError,
    repos::access::{self as access_repo, PermissionScope},
};

/// Permission domains a key may NEVER hold, regardless of the owner's access. Defense in depth: a
/// leaked or over-broad key must not be able to mint further keys or manage other keys. Matched on
/// the permission's first segment (its domain).
pub const API_KEY_FORBIDDEN_DOMAINS: &[&str] = &["api_keys"];

/// Whether `permission_name` is off-limits for API keys (its domain is in the denylist).
pub fn is_forbidden_for_key(permission_name: &str) -> bool {
    let domain = permission_name.split('.').next().unwrap_or("");
    API_KEY_FORBIDDEN_DOMAINS.contains(&domain)
}

#[cfg(test)]
mod tests {
    use super::is_forbidden_for_key;

    #[test]
    fn key_management_permissions_are_forbidden_on_keys() {
        // A key must never be able to mint or manage keys, however its owner is scoped.
        assert!(is_forbidden_for_key("api_keys.key.create"));
        assert!(is_forbidden_for_key("api_keys.key.read"));
        assert!(is_forbidden_for_key("api_keys.key.delete"));
    }

    #[test]
    fn ordinary_permissions_are_allowed_on_keys() {
        assert!(!is_forbidden_for_key("flow.fca.read"));
        assert!(!is_forbidden_for_key("events.rate.update"));
        assert!(!is_forbidden_for_key("")); // empty domain is not denylisted
    }
}

/// The distinct permission names a key was granted (raw, before capping against the owner).
pub async fn fetch_key_permission_names(
    pool: &PgPool,
    api_key_id: &str,
) -> Result<Vec<String>, ApiError> {
    sqlx::query_scalar::<_, String>(
        "select distinct permission_name from access.api_key_permissions \
         where api_key_id = $1 order by permission_name",
    )
    .bind(api_key_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// The scope a key was granted for one permission: `National` if it holds an unscoped (NULL-ARTCC)
/// grant for it, otherwise the set of ARTCC ids from its scoped grants (empty if none).
pub async fn key_granted_scope(
    pool: &PgPool,
    api_key_id: &str,
    permission_name: &str,
) -> Result<PermissionScope, ApiError> {
    let rows = sqlx::query_scalar::<_, Option<String>>(
        "select artcc_id from access.api_key_permissions \
         where api_key_id = $1 and permission_name = $2",
    )
    .bind(api_key_id)
    .bind(permission_name)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;

    if rows.iter().any(Option::is_none) {
        Ok(PermissionScope::National)
    } else {
        Ok(PermissionScope::Facilities(
            rows.into_iter().flatten().collect(),
        ))
    }
}

/// Validate that every requested `(permission, artcc)` grant is within `owner`'s LIVE authority and
/// not denylisted. Rejects (a) denylisted domains, (b) permissions the owner doesn't effectively
/// hold (honoring explicit denies via the effective view), and (c) a scope the owner can't reach —
/// requesting national (`artcc = None`) requires the owner to hold it nationally. This is the
/// no-privilege-escalation gate at key creation/edit time; the request-time cap enforces it again.
pub async fn validate_subset(
    pool: &PgPool,
    owner_user_id: &str,
    requested: &[(String, Option<String>)],
) -> Result<(), ApiError> {
    let owner_names: HashSet<String> =
        access_repo::fetch_user_permission_names(pool, owner_user_id)
            .await?
            .into_iter()
            .collect();

    for (permission_name, artcc_id) in requested {
        if is_forbidden_for_key(permission_name) {
            return Err(ApiError::BadRequest);
        }
        if !owner_names.contains(permission_name) {
            return Err(ApiError::Forbidden);
        }
        let scope = access_repo::permission_scope(pool, owner_user_id, permission_name).await?;
        if !scope.allows(artcc_id.as_deref()) {
            return Err(ApiError::Forbidden);
        }
    }
    Ok(())
}
