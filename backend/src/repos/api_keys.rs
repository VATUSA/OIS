//! User-owned API keys (personal access tokens). This module owns the key's *authority* side:
//! the granted (permission, scope) subset, the denylist, and the "requested set ⊆ owner's live
//! access" validation used when a key is created or edited. The capped effective set (key grants ∩
//! owner's current access) is assembled in `auth::acl::fetch_api_key_access`.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::{
    errors::ApiError,
    models::{ApiKeyBody, ApiKeyPermissionBody},
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

#[cfg(test)]
mod validate_subset_tests {
    use sqlx::PgPool;

    use super::validate_subset;
    use crate::errors::ApiError;
    use crate::scope_test_support::{grant, seed_user};

    const PERM: &str = "flow.surface_data.update";

    fn req(perm: &str, artcc: Option<&str>) -> Vec<(String, Option<String>)> {
        vec![(perm.to_string(), artcc.map(str::to_string))]
    }

    async fn deny(pool: &PgPool, user_id: &str, perm: &str) {
        sqlx::query(
            "insert into access.user_permissions (user_id, permission_name, granted) \
             values ($1, $2, false)",
        )
        .bind(user_id)
        .bind(perm)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn assign_role(pool: &PgPool, user_id: &str, role: &str) {
        sqlx::query("insert into access.user_roles (user_id, role_name) values ($1, $2)")
            .bind(user_id)
            .bind(role)
            .execute(pool)
            .await
            .unwrap();
    }

    #[sqlx::test]
    async fn forbidden_domain_is_rejected_even_when_the_owner_holds_it(pool: PgPool) {
        let user = seed_user(&pool).await;
        grant(&pool, &user, "api_keys.key.create", None).await;
        let r = validate_subset(&pool, &user, &req("api_keys.key.create", None)).await;
        assert!(matches!(r, Err(ApiError::BadRequest)));
    }

    #[sqlx::test]
    async fn a_permission_the_owner_does_not_hold_is_forbidden(pool: PgPool) {
        let user = seed_user(&pool).await;
        let r = validate_subset(&pool, &user, &req(PERM, None)).await;
        assert!(matches!(r, Err(ApiError::Forbidden)));
    }

    #[sqlx::test]
    async fn a_role_grant_removed_by_an_explicit_deny_is_forbidden(pool: PgPool) {
        let user = seed_user(&pool).await;
        // USER grants ace.requests.create nationally — accepted before the deny (positive control).
        assign_role(&pool, &user, "USER").await;
        let perm = req("ace.requests.create", None);
        assert!(validate_subset(&pool, &user, &perm).await.is_ok());

        deny(&pool, &user, "ace.requests.create").await;
        let r = validate_subset(&pool, &user, &perm).await;
        assert!(matches!(r, Err(ApiError::Forbidden)));
    }

    #[sqlx::test]
    async fn an_artcc_scoped_grant_only_allows_that_artcc(pool: PgPool) {
        let user = seed_user(&pool).await;
        grant(&pool, &user, PERM, Some("ZDC")).await;
        assert!(matches!(
            validate_subset(&pool, &user, &req(PERM, None)).await,
            Err(ApiError::Forbidden)
        ));
        assert!(
            validate_subset(&pool, &user, &req(PERM, Some("ZDC")))
                .await
                .is_ok()
        );
        assert!(matches!(
            validate_subset(&pool, &user, &req(PERM, Some("ZNY"))).await,
            Err(ApiError::Forbidden)
        ));
    }

    #[sqlx::test]
    async fn an_in_bounds_request_is_accepted(pool: PgPool) {
        let user = seed_user(&pool).await;
        grant(&pool, &user, PERM, None).await;
        assign_role(&pool, &user, "USER").await;
        let requested = vec![
            (PERM.to_string(), Some("ZNY".to_string())),
            ("ace.requests.create".to_string(), None),
        ];
        assert!(validate_subset(&pool, &user, &requested).await.is_ok());
    }
}

// --- CRUD ---

#[derive(sqlx::FromRow)]
struct ApiKeyRow {
    id: String,
    name: String,
    description: Option<String>,
    prefix: String,
    status: String,
    owner_cid: Option<i64>,
    owner_display_name: Option<String>,
    expires_at: Option<DateTime<Utc>>,
    last_used_at: Option<DateTime<Utc>>,
    last_used_ip: Option<String>,
    revoked_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

const SELECT: &str = "select k.id, k.name, k.description, k.prefix, k.status, \
    u.cid as owner_cid, u.display_name as owner_display_name, \
    k.expires_at, k.last_used_at, k.last_used_ip::text as last_used_ip, k.revoked_at, k.created_at \
    from access.api_keys k join identity.users u on u.id = k.owner_user_id";

async fn row_into_body(pool: &PgPool, row: ApiKeyRow) -> Result<ApiKeyBody, ApiError> {
    let permissions = get_key_permissions(pool, &row.id).await?;
    Ok(ApiKeyBody {
        id: row.id,
        name: row.name,
        description: row.description,
        prefix: row.prefix,
        status: row.status,
        owner_cid: row.owner_cid,
        owner_display_name: row.owner_display_name,
        permissions,
        expires_at: row.expires_at,
        last_used_at: row.last_used_at,
        last_used_ip: row.last_used_ip,
        revoked_at: row.revoked_at,
        created_at: row.created_at,
    })
}

async fn rows_into_bodies(
    pool: &PgPool,
    rows: Vec<ApiKeyRow>,
) -> Result<Vec<ApiKeyBody>, ApiError> {
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_into_body(pool, row).await?);
    }
    Ok(out)
}

/// The granted `(permission, artcc)` subset of a key, as body rows (national = `artcc_id` null).
pub async fn get_key_permissions(
    pool: &PgPool,
    api_key_id: &str,
) -> Result<Vec<ApiKeyPermissionBody>, ApiError> {
    let rows = sqlx::query_as::<_, (String, Option<String>)>(
        "select permission_name, artcc_id from access.api_key_permissions \
         where api_key_id = $1 order by permission_name, artcc_id nulls first",
    )
    .bind(api_key_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(rows
        .into_iter()
        .map(|(permission, artcc_id)| ApiKeyPermissionBody {
            permission,
            artcc_id,
        })
        .collect())
}

/// The owner user id of a key (for ownership checks), or None if the key doesn't exist.
pub async fn fetch_key_owner(pool: &PgPool, api_key_id: &str) -> Result<Option<String>, ApiError> {
    sqlx::query_scalar::<_, String>("select owner_user_id from access.api_keys where id = $1")
        .bind(api_key_id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

pub async fn list_keys_for_owner(
    pool: &PgPool,
    owner_user_id: &str,
) -> Result<Vec<ApiKeyBody>, ApiError> {
    let rows = sqlx::query_as::<_, ApiKeyRow>(&format!(
        "{SELECT} where k.owner_user_id = $1 order by k.created_at desc"
    ))
    .bind(owner_user_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    rows_into_bodies(pool, rows).await
}

/// All keys (admin), optionally filtered to one owner CID.
pub async fn list_all_keys(
    pool: &PgPool,
    owner_cid: Option<i64>,
) -> Result<Vec<ApiKeyBody>, ApiError> {
    let rows = sqlx::query_as::<_, ApiKeyRow>(&format!(
        "{SELECT} where ($1::bigint is null or u.cid = $1) order by k.created_at desc"
    ))
    .bind(owner_cid)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    rows_into_bodies(pool, rows).await
}

pub async fn get_key(pool: &PgPool, id: &str) -> Result<Option<ApiKeyBody>, ApiError> {
    let row = sqlx::query_as::<_, ApiKeyRow>(&format!("{SELECT} where k.id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    match row {
        Some(row) => Ok(Some(row_into_body(pool, row).await?)),
        None => Ok(None),
    }
}

async fn insert_permissions(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    api_key_id: &str,
    permissions: &[(String, Option<String>)],
) -> Result<(), ApiError> {
    for (permission_name, artcc_id) in permissions {
        sqlx::query(
            "insert into access.api_key_permissions (api_key_id, permission_name, artcc_id) \
             values ($1, $2, $3) \
             on conflict (api_key_id, permission_name, coalesce(artcc_id, '')) do nothing",
        )
        .bind(api_key_id)
        .bind(permission_name)
        .bind(artcc_id)
        .execute(&mut **tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    }
    Ok(())
}

/// Create a key and its permission grants in one transaction. Returns the new key id.
#[allow(clippy::too_many_arguments)]
pub async fn create_api_key(
    pool: &PgPool,
    owner_user_id: &str,
    name: &str,
    description: Option<&str>,
    prefix: &str,
    secret_hash: &str,
    expires_at: Option<DateTime<Utc>>,
    permissions: &[(String, Option<String>)],
) -> Result<String, ApiError> {
    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    let id = sqlx::query_scalar::<_, String>(
        "insert into access.api_keys \
             (owner_user_id, name, description, prefix, secret_hash, expires_at) \
         values ($1, $2, $3, $4, $5, $6) returning id",
    )
    .bind(owner_user_id)
    .bind(name)
    .bind(description)
    .bind(prefix)
    .bind(secret_hash)
    .bind(expires_at)
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| ApiError::Internal)?;

    insert_permissions(&mut tx, &id, permissions).await?;
    tx.commit().await.map_err(|_| ApiError::Internal)?;
    Ok(id)
}

/// Replace a key's secret (rotate). Returns false if the key doesn't exist.
pub async fn rotate_key(
    pool: &PgPool,
    id: &str,
    prefix: &str,
    secret_hash: &str,
) -> Result<bool, ApiError> {
    let result =
        sqlx::query("update access.api_keys set prefix = $2, secret_hash = $3 where id = $1")
            .bind(id)
            .bind(prefix)
            .bind(secret_hash)
            .execute(pool)
            .await
            .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

/// Disable a key (revoke it without deleting the record). Returns false if absent.
pub async fn disable_key(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "update access.api_keys set status = 'disabled', revoked_at = now() \
         where id = $1 and status <> 'disabled'",
    )
    .bind(id)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

/// Permanently delete a key (its permission grants cascade). Returns false if absent.
pub async fn delete_key(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    let result = sqlx::query("delete from access.api_keys where id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

/// Replace a key's permission grants with `permissions`.
pub async fn replace_key_permissions(
    pool: &PgPool,
    id: &str,
    permissions: &[(String, Option<String>)],
) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    sqlx::query("delete from access.api_key_permissions where api_key_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    insert_permissions(&mut tx, id, permissions).await?;
    tx.commit().await.map_err(|_| ApiError::Internal)?;
    Ok(())
}
