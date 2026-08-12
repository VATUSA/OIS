//! Access-control queries: session/service-account resolution, effective permissions,
//! and the login-time role/permission reconciliation.

use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};

use crate::{
    auth::{
        acl::PermissionPath,
        context::{CurrentServiceAccount, CurrentUser},
    },
    errors::ApiError,
};

pub fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex_encode(&hasher.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub async fn find_current_user_by_session_token(
    pool: &PgPool,
    session_token: &str,
) -> Result<Option<CurrentUser>, ApiError> {
    sqlx::query_as::<_, CurrentUser>(
        r#"
        select
            u.id,
            u.cid,
            coalesce(u.email::text, '') as email,
            u.display_name,
            u.rating,
            pr.primary_role
        from identity.sessions s
        join identity.users u on u.id = s.user_id
        left join access.v_user_primary_role pr on pr.user_id = u.id
        where s.session_token = $1
          and s.revoked_at is null
          and s.expires_at > now()
        "#,
    )
    .bind(session_token)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn find_current_service_account_by_bearer_token(
    pool: &PgPool,
    bearer_token: &str,
) -> Result<Option<CurrentServiceAccount>, ApiError> {
    let token_hash = sha256_hex(bearer_token);

    let account = sqlx::query_as::<_, CurrentServiceAccount>(
        r#"
        select sa.id, sa.key, sa.name
        from access.service_account_credentials sac
        join access.service_accounts sa on sa.id = sac.service_account_id
        where sac.secret_hash = $1
          and sac.revoked_at is null
          and (sac.expires_at is null or sac.expires_at > now())
          and sa.status = 'active'
        order by sac.created_at desc
        limit 1
        "#,
    )
    .bind(&token_hash)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)?;

    if let Some(account) = account.as_ref() {
        sqlx::query(
            "update access.service_account_credentials set last_used_at = now() \
             where service_account_id = $1 and secret_hash = $2",
        )
        .bind(&account.id)
        .bind(token_hash)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    }

    Ok(account)
}

pub async fn fetch_user_role_names(pool: &PgPool, user_id: &str) -> Result<Vec<String>, ApiError> {
    sqlx::query_scalar::<_, String>(
        "select distinct role_name from access.user_roles where user_id = $1 order by role_name",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn fetch_user_permission_names(
    pool: &PgPool,
    user_id: &str,
) -> Result<Vec<String>, ApiError> {
    sqlx::query_scalar::<_, String>(
        "select permission_name from access.v_effective_user_permissions \
         where user_id = $1 order by permission_name",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn fetch_service_account_role_names(
    pool: &PgPool,
    service_account_id: &str,
) -> Result<Vec<String>, ApiError> {
    sqlx::query_scalar::<_, String>(
        "select distinct role_name from access.service_account_roles \
         where service_account_id = $1 and (ends_at is null or ends_at > now()) order by role_name",
    )
    .bind(service_account_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn fetch_service_account_permission_names(
    pool: &PgPool,
    service_account_id: &str,
) -> Result<Vec<String>, ApiError> {
    sqlx::query_scalar::<_, String>(
        r#"
        select distinct rp.permission_name
        from access.service_account_roles sar
        join access.role_permissions rp on rp.role_name = sar.role_name
        where sar.service_account_id = $1
          and (sar.ends_at is null or sar.ends_at > now())
        order by rp.permission_name
        "#,
    )
    .bind(service_account_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub fn permission_names_to_permissions(
    permission_names: Vec<String>,
) -> Result<Vec<PermissionPath>, ApiError> {
    let mut permissions = Vec::with_capacity(permission_names.len());
    for name in permission_names {
        let Some(permission) = PermissionPath::from_db_value(&name) else {
            tracing::error!(permission_name = %name, "invalid permission value in database");
            return Err(ApiError::Internal);
        };
        permissions.push(permission);
    }
    Ok(permissions)
}

pub async fn find_user_id_by_cid(pool: &PgPool, cid: i64) -> Result<Option<String>, ApiError> {
    sqlx::query_scalar::<_, String>("select id from identity.users where cid = $1")
        .bind(cid)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

pub async fn find_current_user_by_cid(
    pool: &PgPool,
    cid: i64,
) -> Result<Option<CurrentUser>, ApiError> {
    sqlx::query_as::<_, CurrentUser>(
        r#"
        select
            u.id,
            u.cid,
            coalesce(u.email::text, '') as email,
            u.display_name,
            u.rating,
            pr.primary_role
        from identity.users u
        left join access.v_user_primary_role pr on pr.user_id = u.id
        where u.cid = $1
        "#,
    )
    .bind(cid)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Roles a staff user may assign through the access editor. Excludes SERVER_ADMIN
/// (env-bootstrapped only) and the machine roles (BOT / SERVICE_APP). Kept in sync
/// with crates/ois-core/src/catalog.rs.
pub const ASSIGNABLE_USER_ROLES: &[&str] = &[
    "USER",
    "VATUSA_STAFF",
    "EVENTS_NATIONAL",
    "TMU_NATIONAL",
    "ACE_NATIONAL",
    "WEB_TEAM",
    "ATM",
    "DATM",
    "TA",
    "EC",
    "AEC",
    "WM",
    "FE",
    "INS",
    "MTR",
];

/// All permission names in the catalog (the assignable set for the editor).
pub async fn fetch_access_catalog_names(pool: &PgPool) -> Result<Vec<String>, ApiError> {
    sqlx::query_scalar::<_, String>("select name from access.permissions order by name")
        .fetch_all(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// All role names in the catalog.
pub async fn fetch_role_names(pool: &PgPool) -> Result<Vec<String>, ApiError> {
    sqlx::query_scalar::<_, String>("select name from access.roles order by name")
        .fetch_all(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// A user's national (unscoped) direct permission grants — the set the national
/// editor owns. Facility-scoped grants (artcc_id not null) are left untouched.
pub async fn fetch_user_direct_permission_names(
    pool: &PgPool,
    user_id: &str,
) -> Result<Vec<String>, ApiError> {
    sqlx::query_scalar::<_, String>(
        "select permission_name from access.user_permissions \
         where user_id = $1 and granted = true and artcc_id is null order by permission_name",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Grants or revokes a single national (unscoped) role for a user. Only touches
/// national grants so facility-scoped role assignments are preserved.
pub async fn set_user_role_manual(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
    role_name: &str,
    held: bool,
) -> Result<(), ApiError> {
    set_user_role_manual_scoped(tx, user_id, role_name, held, None).await
}

/// Grants or revokes a single role at one scope (`artcc_id = None` national). Only
/// touches that scope; other scopes' assignments are preserved.
pub async fn set_user_role_manual_scoped(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
    role_name: &str,
    held: bool,
    artcc_id: Option<&str>,
) -> Result<(), ApiError> {
    if held {
        sqlx::query(
            r#"
            insert into access.user_roles (user_id, role_name, artcc_id)
            select $1, $2, $3
            where not exists (
                select 1 from access.user_roles
                where user_id = $1 and role_name = $2 and artcc_id is not distinct from $3
            )
            "#,
        )
        .bind(user_id)
        .bind(role_name)
        .bind(artcc_id)
        .execute(&mut **tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    } else {
        sqlx::query(
            "delete from access.user_roles \
             where user_id = $1 and role_name = $2 and artcc_id is not distinct from $3",
        )
        .bind(user_id)
        .bind(role_name)
        .bind(artcc_id)
        .execute(&mut **tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    }
    Ok(())
}

/// Grants SERVER_ADMIN (national scope) if not already held. Idempotent.
pub async fn assign_server_admin(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        r#"
        insert into access.user_roles (user_id, role_name)
        select $1, 'SERVER_ADMIN'
        where not exists (
            select 1 from access.user_roles
            where user_id = $1 and role_name = 'SERVER_ADMIN' and artcc_id is null
        )
        "#,
    )
    .bind(user_id)
    .execute(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(())
}

/// Revokes SERVER_ADMIN. Returns true if a row was actually removed (a demotion).
pub async fn revoke_server_admin(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "delete from access.user_roles where user_id = $1 and role_name = 'SERVER_ADMIN'",
    )
    .bind(user_id)
    .execute(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

/// All direct permission grants (granted = true), as `(artcc_id, permission_name)`.
/// `artcc_id = None` is national. Ordered national-first then by name.
pub async fn fetch_user_direct_grants(
    pool: &PgPool,
    user_id: &str,
) -> Result<Vec<(Option<String>, String)>, ApiError> {
    sqlx::query_as::<_, (Option<String>, String)>(
        "select artcc_id, permission_name from access.user_permissions \
         where user_id = $1 and granted = true \
         order by artcc_id nulls first, permission_name",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// All role grants, as `(artcc_id, role_name)`. `artcc_id = None` is national.
pub async fn fetch_user_role_grants(
    pool: &PgPool,
    user_id: &str,
) -> Result<Vec<(Option<String>, String)>, ApiError> {
    sqlx::query_as::<_, (Option<String>, String)>(
        "select artcc_id, role_name from access.user_roles \
         where user_id = $1 order by artcc_id nulls first, role_name",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Replaces a user's national (unscoped) direct permission grants with `names`.
pub async fn replace_user_permissions(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
    names: &[String],
) -> Result<(), ApiError> {
    replace_user_permissions_scoped(tx, user_id, None, names).await
}

/// Replaces the direct permission grants at one scope (`artcc_id = None` national)
/// with `names`. Deletes everything at that scope first (denies included), then
/// inserts `granted = true` rows. Other scopes are untouched.
pub async fn replace_user_permissions_scoped(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
    artcc_id: Option<&str>,
    names: &[String],
) -> Result<(), ApiError> {
    sqlx::query(
        "delete from access.user_permissions \
         where user_id = $1 and artcc_id is not distinct from $2",
    )
    .bind(user_id)
    .bind(artcc_id)
    .execute(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)?;

    for name in names {
        sqlx::query(
            "insert into access.user_permissions (user_id, permission_name, granted, artcc_id) \
             values ($1, $2, true, $3)",
        )
        .bind(user_id)
        .bind(name)
        .bind(artcc_id)
        .execute(&mut **tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    }
    Ok(())
}

/// Ensures the user has an audit actor row so their actions attribute to them.
pub async fn ensure_user_actor(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
    display_name: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        r#"
        insert into access.actors (actor_type, user_id, display_name)
        select 'user', $1, $2
        where not exists (
            select 1 from access.actors where actor_type = 'user' and user_id = $1
        )
        "#,
    )
    .bind(user_id)
    .bind(display_name)
    .execute(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(())
}
