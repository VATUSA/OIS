//! Access-control queries: session/service-account resolution, effective permissions,
//! and the login-time role/permission reconciliation.

use std::collections::{HashMap, HashSet};

use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};

use crate::{
    auth::{
        acl::PermissionPath,
        context::{CurrentApiKey, CurrentServiceAccount, CurrentUser},
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
        // At most once a minute (#588), for the same reason as an API key's `last_used_at` below.
        sqlx::query(
            "update access.service_account_credentials set last_used_at = now() \
             where service_account_id = $1 and secret_hash = $2 \
               and (last_used_at is null or last_used_at < now() - interval '1 minute')",
        )
        .bind(&account.id)
        .bind(token_hash)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    }

    Ok(account)
}

/// Resolve an `ois_pat_…` bearer token to its API key, if active/unrevoked/unexpired and the owner
/// is still an active user. Updates `last_used_at`/`last_used_ip` on a hit, at most once a minute. The key's *authority* is
/// resolved separately and capped by the owner — see `repos::api_keys` and `auth::principal`.
pub async fn find_current_api_key_by_bearer_token(
    pool: &PgPool,
    bearer_token: &str,
    client_ip: Option<&str>,
) -> Result<Option<CurrentApiKey>, ApiError> {
    let token_hash = sha256_hex(bearer_token);

    let key = sqlx::query_as::<_, CurrentApiKey>(
        r#"
        select k.id, k.owner_user_id, k.prefix, k.name
        from access.api_keys k
        join identity.users u on u.id = k.owner_user_id
        where k.secret_hash = $1
          and k.status = 'active'
          and k.revoked_at is null
          and (k.expires_at is null or k.expires_at > now())
        limit 1
        "#,
    )
    .bind(&token_hash)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)?;

    if let Some(key) = key.as_ref() {
        // `last_used_ip` is inet; a malformed forwarded header simply leaves it null.
        //
        // At most once a minute (#588). This runs before the rate limiter can refuse the request (the
        // limiter needs the resolved key to pick its bucket), so an unconditional write would let a key
        // polling far over its limit still write this row on every refused request, queueing on its row
        // lock in the pool everyone shares. "Last used" is accurate to the minute.
        sqlx::query(
            "update access.api_keys set last_used_at = now(), \
             last_used_ip = coalesce($2::inet, last_used_ip) where id = $1 \
               and (last_used_at is null or last_used_at < now() - interval '1 minute')",
        )
        .bind(&key.id)
        .bind(client_ip)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    }

    Ok(key)
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

/// The names a user effectively holds — the coarse answer `RequirePermission<P>` needs.
///
/// Derived from [`fetch_effective_permissions`] rather than read straight off the view, because
/// the view now also emits deny rows and carries scope; a name survives here when the user holds
/// it at **some** scope. That is the documented contract of the coarse gate: it answers "do you
/// hold this anywhere" (401 if not), and the handler enforces the ARTCC (403). See
/// `docs/architecture/permissions.md`.
pub async fn fetch_user_permission_names(
    pool: &PgPool,
    user_id: &str,
) -> Result<Vec<String>, ApiError> {
    let mut names: Vec<String> = fetch_effective_permissions(pool, user_id)
        .await?
        .into_iter()
        .filter(|(_, scope)| !scope.is_empty())
        .map(|(name, _)| name)
        .collect();
    names.sort(); // the old view read was `order by permission_name`; callers may rely on it
    Ok(names)
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

/// A service account's scope for `permission_name`, from its live roles (#583). National only for a
/// role held with no ARTCC; otherwise the ARTCCs its roles name. The same "live" test as
/// [`fetch_service_account_permission_names`], so the scope never covers a role the gate ignores.
pub async fn service_account_permission_scope(
    pool: &PgPool,
    service_account_id: &str,
    permission_name: &str,
) -> Result<PermissionScope, ApiError> {
    let artccs: Vec<Option<String>> = sqlx::query_scalar(
        "select distinct sar.artcc_id
         from access.service_account_roles sar
         join access.role_permissions rp on rp.role_name = sar.role_name
         where sar.service_account_id = $1 and rp.permission_name = $2
           and (sar.ends_at is null or sar.ends_at > now())",
    )
    .bind(service_account_id)
    .bind(permission_name)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    if artccs.iter().any(Option::is_none) {
        return Ok(PermissionScope::national());
    }
    Ok(PermissionScope::Facilities(
        artccs.into_iter().flatten().collect(),
    ))
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

pub async fn user_display_name(pool: &PgPool, user_id: &str) -> Result<Option<String>, ApiError> {
    sqlx::query_scalar::<_, String>("select display_name from identity.users where id = $1")
        .bind(user_id)
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

/// Positional roles a staff user may assign through the access editor. Excludes
/// SERVER_ADMIN (env-bootstrapped only), the baseline USER role, and the machine roles
/// (BOT / SERVICE_APP). Kept in sync with crates/ois-core/src/catalog.rs and the DB
/// role catalog (migration 0007).
pub const ASSIGNABLE_USER_ROLES: &[&str] = &[
    "VATUSA_STAFF",
    "EVENTS_TEAM",
    "EC",
    "AEC",
    "ACE",
    "NTMO",
    "DCC_STAFF",
];

/// All permission names in the catalog (the assignable set for the editor).
pub async fn fetch_access_catalog_names(pool: &PgPool) -> Result<Vec<String>, ApiError> {
    sqlx::query_scalar::<_, String>("select name from access.permissions order by name")
        .fetch_all(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// Groups whose membership or contents code depends on, so the group editor must not touch them
/// (VATUSA/OIS#545).
///
/// Protected by **name**, not by `access.roles.is_system`. That column exists (`0003_access.sql:9`)
/// but defaults to `true` and every migration inserts `(name, description)` only, so all eleven rows
/// claim to be system roles — enforcing on it would freeze every group. A constant is also honest
/// about *why* these four are protected: each literal is referenced by code, and an admin cannot flip
/// a constant.
///
/// - `SERVER_ADMIN` — `is_server_admin`, the env bootstrap, and the view's cross join.
/// - `USER` — granted at login and carries the signed-in baseline (#544).
/// - `BOT` / `SERVICE_APP` — back service accounts, whose *only* permission source is
///   `role_permissions`, with no cache: emptying `BOT` stops the Discord bot on its next request.
pub const SYSTEM_ROLES: &[&str] = &["SERVER_ADMIN", "USER", "BOT", "SERVICE_APP"];

/// Whether `role_name` is a protected system group.
pub fn is_system_role(role_name: &str) -> bool {
    SYSTEM_ROLES.contains(&role_name)
}

/// Groups a staff user may be assigned, read from the catalogue rather than a constant.
///
/// Replaces [`ASSIGNABLE_USER_ROLES`] at its call sites (#545). The constant could not see a group
/// created through the editor, so a new group was assignable to nobody until someone edited Rust and
/// deployed — which made "create a group, no deploy" untrue. Same reasoning as
/// [`fetch_service_account_assignable_roles`]: deriving the list from the DB is what stops the picker
/// and the validator disagreeing.
pub async fn fetch_assignable_role_names(pool: &PgPool) -> Result<Vec<String>, ApiError> {
    Ok(fetch_role_names(pool)
        .await?
        .into_iter()
        .filter(|name| !is_system_role(name))
        .collect())
}

/// One group as the editor lists it: what it grants, and how many principals hold it.
#[derive(Debug, sqlx::FromRow)]
pub struct GroupRow {
    pub name: String,
    pub description: Option<String>,
    pub permission_count: i64,
    pub user_count: i64,
    pub service_account_count: i64,
}

/// The group projection, shared so the list and the single-row read cannot disagree.
const GROUP_SELECT: &str = r#"
    select
        r.name,
        r.description,
        (select count(*) from access.role_permissions rp where rp.role_name = r.name)
            as permission_count,
        (select count(distinct ur.user_id) from access.user_roles ur where ur.role_name = r.name)
            as user_count,
        (select count(distinct sar.service_account_id) from access.service_account_roles sar
            where sar.role_name = r.name) as service_account_count
    from access.roles r
"#;

/// Every group with its permission count and holder counts.
///
/// The counts are what make the delete guard and the blast radius legible: editing a group changes
/// every holder at once, and a service-account holder is one a user-facing editor would not otherwise
/// show.
pub async fn fetch_groups(pool: &PgPool) -> Result<Vec<GroupRow>, ApiError> {
    sqlx::query_as::<_, GroupRow>(&format!("{GROUP_SELECT} order by r.name"))
        .fetch_all(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// One group, or `None` when it does not exist.
///
/// Separate from [`fetch_groups`] rather than filtering its result in Rust: the handlers need a single
/// group four times over, and scanning every role with three correlated subqueries to pick one row is
/// work for nothing.
pub async fn fetch_group(pool: &PgPool, role_name: &str) -> Result<Option<GroupRow>, ApiError> {
    sqlx::query_as::<_, GroupRow>(&format!("{GROUP_SELECT} where r.name = $1"))
        .bind(role_name)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// Every group's permissions, as `(role_name, permission_name)`.
///
/// One query for the whole listing. Asking per group was an N+1 — twelve round trips to render eleven
/// groups — and the set is small enough that grouping in Rust is free.
pub async fn fetch_all_group_permissions(pool: &PgPool) -> Result<Vec<(String, String)>, ApiError> {
    sqlx::query_as::<_, (String, String)>(
        "select role_name, permission_name from access.role_permissions \
         order by role_name, permission_name",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// The permission names a group grants. Empty for a group that bundles nothing.
pub async fn fetch_group_permissions(
    pool: &PgPool,
    role_name: &str,
) -> Result<Vec<String>, ApiError> {
    sqlx::query_scalar::<_, String>(
        "select permission_name from access.role_permissions          where role_name = $1 order by permission_name",
    )
    .bind(role_name)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// One holder of a group, at one scope.
///
/// A user can appear twice: the unique index is on `(user_id, role_name, coalesce(artcc_id, ''))`, so
/// holding `EC` nationally *and* at ZDC is two rows and shows as two rows. Flattening them would be
/// the same mistake the admin user table's role badges make.
#[derive(Debug, sqlx::FromRow)]
pub struct GroupMemberRow {
    pub cid: i64,
    pub display_name: String,
    pub rating: Option<String>,
    pub artcc_id: Option<String>,
}

/// Match clause shared by the member list and its count, so the two cannot disagree about who is in
/// the page. Mirrors `repos/users.rs`'s `USER_FILTER` rather than inventing a second shape.
const MEMBER_FILTER: &str = "ur.role_name = $1 and u.cid is not null and ( \
    $2 = '' \
    or u.display_name ilike '%' || $2 || '%' \
    or u.full_name ilike '%' || $2 || '%' \
    or cast(u.cid as text) like $2 || '%' )";

/// One page of a group's holders, each with the scope they hold it at.
pub async fn fetch_group_members(
    pool: &PgPool,
    role_name: &str,
    q: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<GroupMemberRow>, ApiError> {
    sqlx::query_as::<_, GroupMemberRow>(&format!(
        "select u.cid, u.display_name, u.rating, ur.artcc_id \
         from access.user_roles ur \
         join identity.users u on u.id = ur.user_id \
         where {MEMBER_FILTER} \
         order by u.display_name asc, ur.artcc_id asc nulls first \
         limit $3 offset $4"
    ))
    .bind(role_name)
    .bind(q)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// How many holder rows a group has, for pagination.
pub async fn count_group_members(pool: &PgPool, role_name: &str, q: &str) -> Result<i64, ApiError> {
    sqlx::query_scalar::<_, i64>(&format!(
        "select count(*) from access.user_roles ur \
         join identity.users u on u.id = ur.user_id where {MEMBER_FILTER}"
    ))
    .bind(role_name)
    .bind(q)
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Replaces a group's permission set. Every holder's access changes on their next request — there is
/// no per-user copy to backfill, which is the point of groups (#542).
pub async fn replace_group_permissions(
    tx: &mut Transaction<'_, Postgres>,
    role_name: &str,
    names: &[String],
) -> Result<(), ApiError> {
    sqlx::query("delete from access.role_permissions where role_name = $1")
        .bind(role_name)
        .execute(&mut **tx)
        .await
        .map_err(|_| ApiError::Internal)?;

    for name in names {
        sqlx::query(
            "insert into access.role_permissions (role_name, permission_name) values ($1, $2)",
        )
        .bind(role_name)
        .bind(name)
        .execute(&mut **tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    }
    Ok(())
}

/// Creates a group. `is_system` is left at its default rather than set: the column is not what
/// protects anything (see [`SYSTEM_ROLES`]), and writing it would imply otherwise.
pub async fn create_group(
    tx: &mut Transaction<'_, Postgres>,
    role_name: &str,
    description: Option<&str>,
) -> Result<(), ApiError> {
    sqlx::query("insert into access.roles (name, description) values ($1, $2)")
        .bind(role_name)
        .bind(description)
        .execute(&mut **tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(())
}

/// Deletes a group. The caller must have established it has no holders: the foreign keys are
/// `on delete cascade` (`0003_access.sql:23,33`), so this would otherwise silently take every
/// membership and every bundled permission with it.
pub async fn delete_group(
    tx: &mut Transaction<'_, Postgres>,
    role_name: &str,
) -> Result<(), ApiError> {
    sqlx::query("delete from access.roles where name = $1")
        .bind(role_name)
        .execute(&mut **tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(())
}

/// All role names in the catalog.
pub async fn fetch_role_names(pool: &PgPool) -> Result<Vec<String>, ApiError> {
    sqlx::query_scalar::<_, String>("select name from access.roles order by name")
        .fetch_all(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// Roles a service account may hold: every role in the catalog except SERVER_ADMIN,
/// which is env-bootstrapped only. This is the single source for both the picker the
/// admin UI renders and `set_service_account_roles`' validation, so the two cannot
/// drift — unlike `ASSIGNABLE_USER_ROLES`, which is the *human* editor's list and
/// deliberately omits the machine roles (BOT / SERVICE_APP).
pub async fn fetch_service_account_assignable_roles(
    pool: &PgPool,
) -> Result<Vec<String>, ApiError> {
    Ok(fetch_role_names(pool)
        .await?
        .into_iter()
        .filter(|name| name != crate::auth::acl::SERVER_ADMIN_ROLE)
        .collect())
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
pub async fn set_user_role(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
    role_name: &str,
    held: bool,
    source: GrantSource,
) -> Result<(), ApiError> {
    set_user_role_scoped(tx, user_id, role_name, held, None, source).await
}

/// Who created a grant, and therefore whose row it is to remove (VATUSA/OIS#547).
///
/// Typed rather than a string so a writer cannot mistype it, and so adding a source is a compile
/// error at every call site rather than a value that silently fails a check constraint at runtime.
///
/// Deliberately **not** consulted by any reader: the `0091` effective-permissions view and
/// [`fetch_effective_permissions`] ignore it, so a grant's authority never depends on who created it
/// and #543's single-resolver property is untouched. This answers only "whose row is this to remove?".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantSource {
    /// An admin set it by hand — the access editor, or a group membership (#546).
    Manual,
    /// The VATUSA sync owns it and may reconcile it away (#548).
    Vatusa,
    /// OIS itself set it: the `SERVER_ADMIN` env reconciliation, and the `USER` baseline group (#544).
    System,
}

impl GrantSource {
    pub fn as_str(self) -> &'static str {
        match self {
            GrantSource::Manual => "manual",
            GrantSource::Vatusa => "vatusa",
            GrantSource::System => "system",
        }
    }
}

/// Grants or revokes a single role at one scope (`artcc_id = None` national). Only
/// touches that scope; other scopes' assignments are preserved.
/// `source` names **whose row this is**, on both the grant and the revoke side. So an admin revoking
/// removes only the manual row and a VATUSA demotion removes only its own — which is how a sync
/// reconciles without touching anything set by hand (#547). The old name claimed "manual" while
/// being the generic primitive; a machine caller was indistinguishable from an admin.
pub async fn set_user_role_scoped(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
    role_name: &str,
    held: bool,
    artcc_id: Option<&str>,
    source: GrantSource,
) -> Result<(), ApiError> {
    if held {
        sqlx::query(
            r#"
            insert into access.user_roles (user_id, role_name, artcc_id, source)
            select $1, $2, $3, $4
            where not exists (
                select 1 from access.user_roles
                where user_id = $1 and role_name = $2 and artcc_id is not distinct from $3
                  and source = $4
            )
            "#,
        )
        .bind(user_id)
        .bind(role_name)
        .bind(artcc_id)
        .bind(source.as_str())
        .execute(&mut **tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    } else {
        sqlx::query(
            "delete from access.user_roles \
             where user_id = $1 and role_name = $2 and artcc_id is not distinct from $3 \
               and source = $4",
        )
        .bind(user_id)
        .bind(role_name)
        .bind(artcc_id)
        .bind(source.as_str())
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
        insert into access.user_roles (user_id, role_name, source)
        select $1, 'SERVER_ADMIN', 'system'
        where not exists (
            select 1 from access.user_roles
            where user_id = $1 and role_name = 'SERVER_ADMIN' and artcc_id is null
              and source = 'system'
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
        // `system`: the row this reconciliation created. A SERVER_ADMIN granted some other way is not
        // this function's to remove (#547).
        "delete from access.user_roles \
         where user_id = $1 and role_name = 'SERVER_ADMIN' and source = 'system'",
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
        // `distinct` for the same reason as `fetch_user_role_grants` (#547).
        "select distinct artcc_id, permission_name from access.user_permissions \
         where user_id = $1 and granted = true \
         order by artcc_id nulls first, permission_name",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// A user's authority for one permission: national (every ARTCC, minus any explicitly denied)
/// or a specific set of ARTCCs.
///
/// `National { except }` exists because a national allow minus a scoped deny is "everywhere but
/// ZDC", which a plain facility set cannot express without enumerating every facility — and
/// enumerating would silently turn a national holder into a scoped one, changing what
/// [`PermissionScope::allows`] answers for a resource with no resolvable owning ARTCC
/// (VATUSA/OIS#543). An empty `except` is plain national.
#[derive(Debug, Clone)]
pub enum PermissionScope {
    National { except: HashSet<String> },
    Facilities(HashSet<String>),
}

impl PermissionScope {
    /// National with no exceptions — the common case, and what `SERVER_ADMIN` always gets.
    pub fn national() -> Self {
        PermissionScope::National {
            except: HashSet::new(),
        }
    }

    /// Whether this is national authority at all, exceptions aside. Callers that need to know
    /// "is this a national reader" (rather than "may they act here") ask this.
    pub fn is_national(&self) -> bool {
        matches!(self, PermissionScope::National { .. })
    }

    /// Whether this scope covers `artcc`. `None` (an airport with no resolvable owning
    /// ARTCC) is editable only at unrestricted national scope — a holder carrying any scoped
    /// deny fails closed there, because there is no ARTCC to test the deny against.
    pub fn allows(&self, artcc: Option<&str>) -> bool {
        match self {
            PermissionScope::National { except } => match artcc {
                Some(a) => !except.contains(a),
                None => except.is_empty(),
            },
            PermissionScope::Facilities(set) => artcc.is_some_and(|a| set.contains(a)),
        }
    }

    /// Covers nothing — an empty facility set. National covers something whatever its
    /// exceptions, since a national deny collapses to `Facilities(∅)` rather than an exception.
    pub fn is_empty(&self) -> bool {
        matches!(self, PermissionScope::Facilities(set) if set.is_empty())
    }

    /// The narrower of two scopes — used to cap an API key at its owner's authority.
    /// Exceptions accumulate; a facility set loses anything the other side excepts.
    pub fn intersect(&self, other: &PermissionScope) -> PermissionScope {
        match (self, other) {
            (PermissionScope::National { except: a }, PermissionScope::National { except: b }) => {
                PermissionScope::National {
                    except: a.union(b).cloned().collect(),
                }
            }
            (PermissionScope::National { except }, PermissionScope::Facilities(set))
            | (PermissionScope::Facilities(set), PermissionScope::National { except }) => {
                PermissionScope::Facilities(set.difference(except).cloned().collect())
            }
            (PermissionScope::Facilities(a), PermissionScope::Facilities(b)) => {
                PermissionScope::Facilities(a.intersection(b).cloned().collect())
            }
        }
    }
}

/// One row of the effective-permissions view: a permission, the scope it applies at
/// (`None` = national), and whether it grants or denies.
struct EffectiveRow {
    permission_name: String,
    artcc_id: Option<String>,
    granted: bool,
}

/// Compose raw (permission, scope, granted) facts into the authority held for each permission.
///
/// The single place the deny rule lives (VATUSA/OIS#543). A deny removes the permission **at its
/// own scope**; a national deny removes it **everywhere**, even where a scoped allow exists —
/// `docs/architecture/permissions.md`'s long-standing promise that an explicit deny beats any
/// allow, now actually implemented. Pure, so the matrix is testable without a database.
fn compose(rows: Vec<EffectiveRow>) -> HashMap<String, PermissionScope> {
    // Per permission: national allow seen, scoped allows, national deny seen, scoped denies.
    struct Facts {
        national_allow: bool,
        national_deny: bool,
        allows: HashSet<String>,
        denies: HashSet<String>,
    }
    let mut facts: HashMap<String, Facts> = HashMap::new();

    for row in rows {
        let entry = facts.entry(row.permission_name).or_insert_with(|| Facts {
            national_allow: false,
            national_deny: false,
            allows: HashSet::new(),
            denies: HashSet::new(),
        });
        match (row.artcc_id, row.granted) {
            (None, true) => entry.national_allow = true,
            (None, false) => entry.national_deny = true,
            (Some(artcc), true) => {
                entry.allows.insert(artcc);
            }
            (Some(artcc), false) => {
                entry.denies.insert(artcc);
            }
        }
    }

    facts
        .into_iter()
        .map(|(name, f)| {
            let scope = if f.national_deny {
                // Beats everything, including a scoped allow.
                PermissionScope::Facilities(HashSet::new())
            } else if f.national_allow {
                PermissionScope::National {
                    except: f.denies.clone(),
                }
            } else {
                PermissionScope::Facilities(f.allows.difference(&f.denies).cloned().collect())
            };
            (name, scope)
        })
        .collect()
}

/// Every permission the user effectively holds, with the scope they hold it at.
///
/// **The one resolver.** Before #543 there were two — a SQL view that honoured denies but dropped
/// `artcc_id`, so a scoped deny revoked nationally, and `permission_scope()` which honoured scope
/// but never read `granted = false`. Both of those are gone; everything funnels through here, so
/// the two dimensions cannot drift apart again.
///
/// One query per user rather than one per permission: callers that previously asked about several
/// permissions in a loop (the API-key cap, the grantable-permissions list) now resolve once.
pub async fn fetch_effective_permissions(
    pool: &PgPool,
    user_id: &str,
) -> Result<HashMap<String, PermissionScope>, ApiError> {
    let rows = sqlx::query_as::<_, (String, Option<String>, bool)>(
        "select permission_name, artcc_id, granted \
         from access.v_effective_user_permissions where user_id = $1",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;

    Ok(compose(
        rows.into_iter()
            .map(|(permission_name, artcc_id, granted)| EffectiveRow {
                permission_name,
                artcc_id,
                granted,
            })
            .collect(),
    ))
}

/// Resolve which ARTCCs a user effectively holds `permission_name` in.
///
/// A thin lookup into [`fetch_effective_permissions`] — it holds no SQL of its own, which is the
/// point of #543. A permission the user does not hold resolves to the empty facility set, so
/// callers keep failing closed.
pub async fn permission_scope(
    pool: &PgPool,
    user_id: &str,
    permission_name: &str,
) -> Result<PermissionScope, ApiError> {
    Ok(fetch_effective_permissions(pool, user_id)
        .await?
        .remove(permission_name)
        .unwrap_or_else(|| PermissionScope::Facilities(HashSet::new())))
}

/// All role grants, as `(artcc_id, role_name)`. `artcc_id = None` is national.
pub async fn fetch_user_role_grants(
    pool: &PgPool,
    user_id: &str,
) -> Result<Vec<(Option<String>, String)>, ApiError> {
    sqlx::query_as::<_, (Option<String>, String)>(
        // `distinct`: since #547 a manual and a synced grant of the same role at the same scope are
        // two rows, and the editor must list it once.
        "select distinct artcc_id, role_name from access.user_roles \
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
    // Grants only. The access editor has no notion of a deny, so replacing a scope must never remove
    // one: it used to delete every row here, which meant saving a user's ZDC scope *unchanged* silently
    // stripped their ZDC denies and widened their access, with no guard ever seeing it (#559).
    sqlx::query(
        // **Only the manual rows.** Before #547 this deleted everything at the scope, so the first
        // admin save after a sync wiped every synced grant — the likeliest silent access-loss bug in
        // the epic. The accepted consequence: an admin cannot un-grant a synced role here; they
        // detach the user from sync (#549).
        "delete from access.user_permissions \
         where user_id = $1 and artcc_id is not distinct from $2 \
            and source = 'manual' and granted is true",
    )
    .bind(user_id)
    .bind(artcc_id)
    .execute(&mut **tx)
    .await
    .map_err(|_| ApiError::Internal)?;

    for name in names {
        // An explicit grant where a *manual* deny exists replaces it, rather than violating the unique
        // index, which 0098 widened to `(user_id, permission_name, coalesce(artcc_id, ''), source)`.
        // Keying the conflict on `source` too means this can only ever collide with the editor's own
        // row: a synced grant at the same scope is a separate row and is left to the sync to reconcile.
        // `enforce_actor_scope` has already required the actor to hold it at this scope.
        sqlx::query(
            "insert into access.user_permissions \
                 (user_id, permission_name, granted, artcc_id, source) \
             values ($1, $2, true, $3, 'manual') \
             on conflict (user_id, permission_name, coalesce(artcc_id, ''), source) \
             do update set granted = true",
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::PermissionScope;

    fn facilities(ids: &[&str]) -> PermissionScope {
        PermissionScope::Facilities(ids.iter().map(|s| s.to_string()).collect())
    }

    /// A provenance helper: make a row that a given source owns. Tests say what they mean, and a
    /// `vatusa` row is otherwise three lines of SQL each time (VATUSA/OIS#547).
    #[cfg(test)]
    async fn seed_grant(
        pool: &sqlx::PgPool,
        user_id: &str,
        permission_name: &str,
        artcc: Option<&str>,
        source: &str,
    ) {
        sqlx::query(
            "insert into access.user_permissions \
                 (user_id, permission_name, granted, artcc_id, source) \
             values ($1, $2, true, $3, $4)",
        )
        .bind(user_id)
        .bind(permission_name)
        .bind(artcc)
        .bind(source)
        .execute(pool)
        .await
        .unwrap();
    }

    #[cfg(test)]
    async fn sources_of(pool: &sqlx::PgPool, user_id: &str) -> Vec<(String, String)> {
        sqlx::query_as::<_, (String, String)>(
            "select permission_name, source from access.user_permissions \
             where user_id = $1 order by permission_name, source",
        )
        .bind(user_id)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    /// AC3, and the one the issue calls the likeliest silent access-loss bug in the epic: before #547
    /// `replace_user_permissions_scoped` deleted *everything* at the scope, so the first admin save
    /// after a sync wiped every synced grant. Against the old unconditional delete this test fails.
    #[sqlx::test]
    async fn an_admin_save_leaves_synced_grants_alone(pool: sqlx::PgPool) {
        let user: String = sqlx::query_scalar(
            "insert into identity.users (full_name, display_name) values ('T', 'T') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        seed_grant(&pool, &user, "events.config.update", None, "vatusa").await;
        seed_grant(&pool, &user, "ace.requests.claim", None, "manual").await;

        // The admin saves a different set entirely — the editor is authoritative over its own rows.
        let mut tx = pool.begin().await.unwrap();
        super::replace_user_permissions_scoped(
            &mut tx,
            &user,
            None,
            &["tmu.ntml.create".to_string()],
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        let rows = sources_of(&pool, &user).await;
        assert_eq!(
            rows,
            vec![
                ("events.config.update".to_string(), "vatusa".to_string()),
                ("tmu.ntml.create".to_string(), "manual".to_string()),
            ],
            "the synced grant must survive an admin save, and the admin's own prior row must not"
        );
    }

    /// AC2: sync reconciles only what it owns. The mirror of the test above — a VATUSA revoke at a
    /// scope leaves a manual grant at that same scope standing.
    #[sqlx::test]
    async fn a_sync_revoke_removes_only_its_own_row(pool: sqlx::PgPool) {
        let user: String = sqlx::query_scalar(
            "insert into identity.users (full_name, display_name) values ('T', 'T') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        seed_grant(&pool, &user, "events.config.update", Some("ZDC"), "vatusa").await;
        seed_grant(&pool, &user, "ace.requests.claim", Some("ZDC"), "manual").await;

        // What a reconciliation does (#548): withdraw this source's rows at the scope.
        sqlx::query(
            "delete from access.user_permissions \
             where user_id = $1 and artcc_id is not distinct from $2 and source = 'vatusa'",
        )
        .bind(&user)
        .bind(Some("ZDC"))
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(
            sources_of(&pool, &user).await,
            vec![("ace.requests.claim".to_string(), "manual".to_string())],
            "a sync revoke must not reach a hand-made grant at the same scope"
        );
    }

    /// AC1 structurally. The migration adds `source` with a default only to backfill, then drops it,
    /// so a writer that forgets fails `not null` instead of silently claiming to be a human grant.
    /// This asserts the schema enforces it rather than trusting every author to remember.
    #[sqlx::test]
    async fn an_insert_without_a_source_is_rejected(pool: sqlx::PgPool) {
        let user: String = sqlx::query_scalar(
            "insert into identity.users (full_name, display_name) values ('T', 'T') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        for table in ["access.user_roles", "access.user_permissions"] {
            let column = if table.ends_with("roles") {
                "role_name"
            } else {
                "permission_name"
            };
            let err = sqlx::query(&format!(
                "insert into {table} (user_id, {column}) values ($1, 'USER')"
            ))
            .bind(&user)
            .execute(&pool)
            .await
            .expect_err("omitting source must fail: the backfill default is dropped");
            assert!(
                err.to_string().contains("source"),
                "{table}: expected a not-null violation naming source, got {err}"
            );
        }

        // And a source outside the three is refused by the check constraint.
        sqlx::query(
            "insert into access.user_roles (user_id, role_name, source) values ($1, 'USER', 'guess')",
        )
        .bind(&user)
        .execute(&pool)
        .await
        .expect_err("an unknown source must violate the check constraint");
    }

    /// AC4: rows that predate the migration read as `manual`.
    ///
    /// `#[sqlx::test]` applies every migration before any test data exists, so the backfill cannot be
    /// observed by inserting afterwards. This puts both tables back how `0098` found them and then runs
    /// **the real migration file** (`include_str!`), not a copy typed into the test. A copy is what this
    /// test used to hold, and changing `0098` to backfill `'vatusa'` left it — and every other test —
    /// green (#547 review). That value matters more than any other here: a wrong backfill marks every
    /// hand-made grant as synced, and the first sync then revokes them as its own.
    ///
    /// Re-runnable because `0098` is written with `if not exists` / `if exists` throughout; dropping
    /// `source` also drops the indexes and check constraint built on it, which the file recreates.
    #[sqlx::test]
    async fn the_backfill_marks_pre_existing_rows_manual(pool: sqlx::PgPool) {
        const MIGRATION_0098: &str = include_str!("../../migrations/0098_grant_provenance.sql");

        let user: String = sqlx::query_scalar(
            "insert into identity.users (full_name, display_name) values ('T', 'T') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        // Put both tables back how 0098 found them, each holding a row with no provenance.
        for table in ["access.user_roles", "access.user_permissions"] {
            sqlx::query(&format!("alter table {table} drop column source"))
                .execute(&pool)
                .await
                .unwrap();
        }
        sqlx::query("insert into access.user_roles (user_id, role_name) values ($1, 'NTMO')")
            .bind(&user)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "insert into access.user_permissions (user_id, permission_name, granted) \
             values ($1, 'events.config.update', true)",
        )
        .bind(&user)
        .execute(&pool)
        .await
        .unwrap();

        sqlx::raw_sql(MIGRATION_0098).execute(&pool).await.unwrap();

        let role_source: String = sqlx::query_scalar(
            "select source from access.user_roles where user_id = $1 and role_name = 'NTMO'",
        )
        .bind(&user)
        .fetch_one(&pool)
        .await
        .unwrap();
        let perm_source: String = sqlx::query_scalar(
            "select source from access.user_permissions \
             where user_id = $1 and permission_name = 'events.config.update'",
        )
        .bind(&user)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            role_source, "manual",
            "an existing role grant backfills as hand-made"
        );
        assert_eq!(
            perm_source, "manual",
            "an existing permission grant backfills as hand-made"
        );

        // …and the default really is gone afterwards, on both tables, which is what makes AC1 the
        // schema's job rather than every author's memory.
        sqlx::query("insert into access.user_roles (user_id, role_name) values ($1, 'USER')")
            .bind(&user)
            .execute(&pool)
            .await
            .expect_err("the backfill default must not survive on user_roles");
        sqlx::query(
            "insert into access.user_permissions (user_id, permission_name, granted) \
             values ($1, 'tmu.program.update', true)",
        )
        .bind(&user)
        .execute(&pool)
        .await
        .expect_err("the backfill default must not survive on user_permissions");
    }

    /// AC2, against the writer the sync will call. The sync itself is #548 and not built yet, but
    /// `set_user_role_scoped` is, and it is where the guarantee lives: the same **role** at the same
    /// **scope**, held both by hand and by sync, must lose only the synced row when the sync revokes.
    ///
    /// The earlier `a_sync_revoke_removes_only_its_own_row` runs a hand-typed delete on
    /// `user_permissions` with two *different* permissions, so it pins none of that — removing
    /// `and source = $4` from this writer left the whole suite green (#547 review).
    #[sqlx::test]
    async fn a_sync_role_revoke_leaves_the_manual_grant_at_the_same_scope(pool: sqlx::PgPool) {
        let user: String = sqlx::query_scalar(
            "insert into identity.users (full_name, display_name) values ('T', 'T') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        let mut tx = pool.begin().await.unwrap();
        for source in [super::GrantSource::Manual, super::GrantSource::Vatusa] {
            super::set_user_role_scoped(&mut tx, &user, "NTMO", true, Some("ZDC"), source)
                .await
                .unwrap();
        }
        tx.commit().await.unwrap();

        let rows = |pool: sqlx::PgPool, user: String| async move {
            sqlx::query_scalar::<_, String>(
                "select source from access.user_roles \
                 where user_id = $1 and role_name = 'NTMO' and artcc_id = 'ZDC' order by source",
            )
            .bind(user)
            .fetch_all(&pool)
            .await
            .unwrap()
        };
        assert_eq!(
            rows(pool.clone(), user.clone()).await,
            vec!["manual".to_string(), "vatusa".to_string()],
            "both sources coexist at one scope"
        );

        let mut tx = pool.begin().await.unwrap();
        super::set_user_role_scoped(
            &mut tx,
            &user,
            "NTMO",
            false,
            Some("ZDC"),
            super::GrantSource::Vatusa,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        assert_eq!(
            rows(pool.clone(), user.clone()).await,
            vec!["manual".to_string()],
            "a sync revoke must remove its own row and leave the hand-made one"
        );
    }

    /// Since a manual and a synced grant of the same role at the same scope are now two rows, the
    /// grant readers must still report it once — otherwise the access editor lists the role twice.
    /// This is the cost the unique-index change buys, and it is pinned rather than remembered.
    #[sqlx::test]
    async fn the_grant_readers_collapse_two_sources_into_one_entry(pool: sqlx::PgPool) {
        let user: String = sqlx::query_scalar(
            "insert into identity.users (full_name, display_name) values ('T', 'T') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        for source in ["manual", "vatusa"] {
            sqlx::query(
                "insert into access.user_roles (user_id, role_name, artcc_id, source) \
                 values ($1, 'EC', 'ZDC', $2)",
            )
            .bind(&user)
            .bind(source)
            .execute(&pool)
            .await
            .unwrap();

            seed_grant(&pool, &user, "ace.requests.claim", Some("ZDC"), source).await;
        }

        assert_eq!(
            super::fetch_user_role_grants(&pool, &user).await.unwrap(),
            vec![(Some("ZDC".to_string()), "EC".to_string())],
            "a role held from two sources is still one entry in the editor"
        );
        assert_eq!(
            super::fetch_user_direct_grants(&pool, &user).await.unwrap(),
            vec![(Some("ZDC".to_string()), "ace.requests.claim".to_string())],
            "and so is a direct permission"
        );
    }

    /// The `SERVER_ADMIN` env reconciliation owns its row as `system`, so neither a sync nor an admin
    /// save can take it away — and its revoke only removes what it granted.
    #[sqlx::test]
    async fn the_server_admin_reconciliation_owns_a_system_row(pool: sqlx::PgPool) {
        let user: String = sqlx::query_scalar(
            "insert into identity.users (full_name, display_name) values ('T', 'T') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        let mut tx = pool.begin().await.unwrap();
        super::assign_server_admin(&mut tx, &user).await.unwrap();
        tx.commit().await.unwrap();

        let source: String = sqlx::query_scalar(
            "select source from access.user_roles \
             where user_id = $1 and role_name = 'SERVER_ADMIN'",
        )
        .bind(&user)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(source, "system");

        // A hand-made SERVER_ADMIN is a separate row, and the reconciliation's revoke leaves it.
        sqlx::query(
            "insert into access.user_roles (user_id, role_name, source) \
             values ($1, 'SERVER_ADMIN', 'manual')",
        )
        .bind(&user)
        .execute(&pool)
        .await
        .expect("a manual row coexists with the system one");

        let mut tx = pool.begin().await.unwrap();
        super::revoke_server_admin(&mut tx, &user).await.unwrap();
        tx.commit().await.unwrap();

        let remaining: Vec<String> = sqlx::query_scalar(
            "select source from access.user_roles \
             where user_id = $1 and role_name = 'SERVER_ADMIN'",
        )
        .bind(&user)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            remaining,
            vec!["manual".to_string()],
            "the env reconciliation must revoke only its own grant"
        );
    }

    #[test]
    fn is_empty_only_for_empty_facility_set() {
        assert!(!PermissionScope::national().is_empty());
        assert!(!facilities(&["ZDC"]).is_empty());
        assert!(facilities(&[]).is_empty());
    }

    #[test]
    fn intersect_national_is_identity() {
        // National ∩ X = X (a key with a national grant is capped to the owner's scope, and vice versa).
        assert!(
            PermissionScope::national()
                .intersect(&PermissionScope::national())
                .is_national()
        );
        assert!(
            PermissionScope::national()
                .intersect(&facilities(&["ZDC", "ZNY"]))
                .allows(Some("ZDC"))
        );
        assert!(
            facilities(&["ZDC"])
                .intersect(&PermissionScope::national())
                .allows(Some("ZDC"))
        );
    }

    #[test]
    fn intersect_facilities_is_the_common_set() {
        let both =
            facilities(&["ZDC", "ZNY", "ZBW"]).intersect(&facilities(&["ZNY", "ZBW", "ZOB"]));
        assert!(both.allows(Some("ZNY")));
        assert!(both.allows(Some("ZBW")));
        assert!(!both.allows(Some("ZDC"))); // owner-only
        assert!(!both.allows(Some("ZOB"))); // key-only
    }

    #[test]
    fn intersect_disjoint_facilities_covers_nothing() {
        // A key scoped to an ARTCC its owner can't reach ends up with no authority — fail closed.
        let none = facilities(&["ZLA"]).intersect(&facilities(&["ZDC"]));
        assert!(none.is_empty());
        assert!(!none.allows(Some("ZLA")));
        assert!(!none.allows(Some("ZDC")));
    }

    /// AC3's real guarantee: the picker's list must contain BOT, or the UI cannot grant the
    /// Discord bot its role (VATUSA/OIS#531, which unblocks #445). ASSIGNABLE_USER_ROLES — the
    /// *human* editor's list, which `/access/catalog` serves — deliberately omits it, and that is
    /// exactly the trap this function exists to avoid.
    #[sqlx::test]
    async fn service_account_roles_offer_bot_but_never_server_admin(pool: sqlx::PgPool) {
        let roles = super::fetch_service_account_assignable_roles(&pool)
            .await
            .unwrap();

        assert!(roles.iter().any(|r| r == "BOT"), "BOT missing: {roles:?}");
        assert!(
            !roles
                .iter()
                .any(|r| r == crate::auth::acl::SERVER_ADMIN_ROLE),
            "SERVER_ADMIN must stay env-bootstrapped only: {roles:?}"
        );
        assert!(
            !super::ASSIGNABLE_USER_ROLES.contains(&"BOT"),
            "if BOT ever becomes user-assignable this function's reason to exist is gone"
        );
    }

    // ---- VATUSA/OIS#543: the one resolver, over both dimensions ----

    fn row(permission: &str, artcc: Option<&str>, granted: bool) -> super::EffectiveRow {
        super::EffectiveRow {
            permission_name: permission.to_string(),
            artcc_id: artcc.map(str::to_string),
            granted,
        }
    }

    fn scope_of(rows: Vec<super::EffectiveRow>) -> PermissionScope {
        super::compose(rows)
            .remove("p")
            .unwrap_or_else(|| PermissionScope::Facilities(HashSet::new()))
    }

    /// AC4 — the matrix the two old resolvers each answered half of. {national, scoped} ×
    /// {grant, deny}, with the role-derived and SERVER_ADMIN arms covered by the DB tests below
    /// (they differ only in which view arm produces the row, not in how it composes).
    #[test]
    fn the_deny_rule_over_the_whole_matrix() {
        // (rows, artcc under test, expected) — `None` artcc is a resource with no owning ARTCC.
        let cases: Vec<(&str, Vec<super::EffectiveRow>, Option<&str>, bool)> = vec![
            ("nothing at all", vec![], Some("ZDC"), false),
            (
                "national allow",
                vec![row("p", None, true)],
                Some("ZDC"),
                true,
            ),
            (
                "national allow, unknown artcc",
                vec![row("p", None, true)],
                None,
                true,
            ),
            (
                "scoped allow, same artcc",
                vec![row("p", Some("ZDC"), true)],
                Some("ZDC"),
                true,
            ),
            // The original bug's mirror: a grant scoped to ZDC must NOT read as national.
            (
                "scoped allow, other artcc",
                vec![row("p", Some("ZDC"), true)],
                Some("ZNY"),
                false,
            ),
            (
                "scoped allow, unknown artcc",
                vec![row("p", Some("ZDC"), true)],
                None,
                false,
            ),
            // A deny at its own scope removes it there.
            (
                "scoped allow + same-scope deny",
                vec![row("p", Some("ZDC"), true), row("p", Some("ZDC"), false)],
                Some("ZDC"),
                false,
            ),
            // AC1, the headline: a deny at ZDC must not reach ZNY.
            (
                "two scoped allows + one scoped deny",
                vec![
                    row("p", Some("ZDC"), true),
                    row("p", Some("ZNY"), true),
                    row("p", Some("ZDC"), false),
                ],
                Some("ZNY"),
                true,
            ),
            // A national deny beats a scoped allow — deny wins at or above its scope.
            (
                "scoped allow + national deny",
                vec![row("p", Some("ZDC"), true), row("p", None, false)],
                Some("ZDC"),
                false,
            ),
            // And a scoped deny carves a hole in a national allow rather than erasing it.
            (
                "national allow + scoped deny, elsewhere",
                vec![row("p", None, true), row("p", Some("ZDC"), false)],
                Some("ZNY"),
                true,
            ),
            (
                "national allow + scoped deny, at the denied artcc",
                vec![row("p", None, true), row("p", Some("ZDC"), false)],
                Some("ZDC"),
                false,
            ),
            // Fail closed: with no ARTCC to test the deny against, a carved national scope
            // cannot be shown to allow the action.
            (
                "national allow + scoped deny, unknown artcc",
                vec![row("p", None, true), row("p", Some("ZDC"), false)],
                None,
                false,
            ),
            (
                "national allow + national deny",
                vec![row("p", None, true), row("p", None, false)],
                Some("ZDC"),
                false,
            ),
        ];

        for (name, rows, artcc, expected) in cases {
            let scope = scope_of(rows);
            assert_eq!(
                scope.allows(artcc),
                expected,
                "{name}: allows({artcc:?}) should be {expected} — got {scope:?}"
            );
        }
    }

    /// A permission denied outright holds nothing, so the coarse gate drops it rather than
    /// admitting the caller and relying on a handler check that may not exist.
    #[test]
    fn a_wholly_denied_permission_is_empty_not_merely_narrow() {
        assert!(scope_of(vec![row("p", None, true), row("p", None, false)]).is_empty());
        assert!(
            scope_of(vec![
                row("p", Some("ZDC"), true),
                row("p", Some("ZDC"), false)
            ])
            .is_empty()
        );
        // But a carved national scope still holds something, so it must not be dropped.
        assert!(!scope_of(vec![row("p", None, true), row("p", Some("ZDC"), false)]).is_empty());
    }

    #[test]
    fn permissions_compose_independently_of_each_other() {
        let resolved = super::compose(vec![
            row("kept", None, true),
            row("dropped", None, true),
            row("dropped", None, false),
        ]);
        assert!(resolved["kept"].is_national());
        assert!(resolved["dropped"].is_empty());
    }

    /// AC1 + AC2 through the real view: a deny scoped to one ARTCC must not revoke the permission
    /// at another, and `permission_scope` must read denies at all — it never did.
    ///
    /// The allow comes from a **role** at two ARTCCs and the deny is a direct row at one of them.
    /// That pairing matters: `access.user_permissions` has a unique index on
    /// `(user_id, permission_name, coalesce(artcc_id, ''))`, so a direct allow and a direct deny
    /// cannot coexist at the same scope — a role-derived allow is the only way to reach the
    /// same-scope conflict, and it is also the shape #542 creates (groups carry the permissions,
    /// a direct deny is the per-user override).
    #[sqlx::test]
    async fn a_scoped_deny_does_not_revoke_the_permission_elsewhere(pool: sqlx::PgPool) {
        let user: String = sqlx::query_scalar(
            "insert into identity.users (full_name, display_name) values ('T', 'T') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("insert into access.roles (name) values ('DENY_TEST') on conflict do nothing")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "insert into access.role_permissions (role_name, permission_name) \
             values ('DENY_TEST', 'events.config.update')",
        )
        .execute(&pool)
        .await
        .unwrap();
        for artcc in ["ZDC", "ZNY"] {
            sqlx::query(
                "insert into access.user_roles (user_id, role_name, artcc_id, source) \
                 values ($1, 'DENY_TEST', $2, 'manual')",
            )
            .bind(&user)
            .bind(artcc)
            .execute(&pool)
            .await
            .unwrap();
        }
        crate::scope_test_support::deny_scoped(&pool, &user, "events.config.update", Some("ZDC"))
            .await;

        let scope = super::permission_scope(&pool, &user, "events.config.update")
            .await
            .unwrap();

        // Against the old name-only anti-join this was a global subtraction: ZNY died with ZDC.
        assert!(
            !scope.allows(Some("ZDC")),
            "the denied ARTCC must be refused"
        );
        assert!(
            scope.allows(Some("ZNY")),
            "a deny at ZDC must not reach ZNY"
        );

        // And the coarse gate still admits them, because they hold it somewhere.
        let names = super::fetch_user_permission_names(&pool, &user)
            .await
            .unwrap();
        assert!(names.iter().any(|n| n == "events.config.update"));
    }

    /// The other half of the rule: a **national** deny beats a scoped allow, so the permission is
    /// gone everywhere and the coarse gate stops admitting the caller at all.
    #[sqlx::test]
    async fn a_national_deny_removes_a_scoped_grant_everywhere(pool: sqlx::PgPool) {
        let user: String = sqlx::query_scalar(
            "insert into identity.users (full_name, display_name) values ('T', 'T') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        crate::scope_test_support::grant(&pool, &user, "events.config.update", Some("ZDC")).await;
        crate::scope_test_support::deny_scoped(&pool, &user, "events.config.update", None).await;

        let scope = super::permission_scope(&pool, &user, "events.config.update")
            .await
            .unwrap();
        assert!(scope.is_empty(), "a national deny beats a scoped allow");
        assert!(!scope.allows(Some("ZDC")));

        let names = super::fetch_user_permission_names(&pool, &user)
            .await
            .unwrap();
        assert!(
            !names.iter().any(|n| n == "events.config.update"),
            "holding it nowhere means the coarse gate must not admit them"
        );
    }

    /// The scoped dimension the view used to drop: a grant at one ARTCC is not national.
    #[sqlx::test]
    async fn a_role_scoped_grant_is_not_national(pool: sqlx::PgPool) {
        let user: String = sqlx::query_scalar(
            "insert into identity.users (full_name, display_name) values ('T', 'T') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("insert into access.roles (name) values ('SCOPE_TEST') on conflict do nothing")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "insert into access.role_permissions (role_name, permission_name) \
             values ('SCOPE_TEST', 'events.config.update')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into access.user_roles (user_id, role_name, artcc_id, source) \
             values ($1, 'SCOPE_TEST', 'ZDC', 'manual')",
        )
        .bind(&user)
        .execute(&pool)
        .await
        .unwrap();

        let scope = super::permission_scope(&pool, &user, "events.config.update")
            .await
            .unwrap();
        assert!(scope.allows(Some("ZDC")));
        assert!(
            !scope.allows(Some("ZNY")),
            "a ZDC-scoped role must not grant at ZNY"
        );
        assert!(!scope.is_national());
    }

    /// SERVER_ADMIN carries the whole catalogue nationally, with no grant rows of its own.
    ///
    /// Only that — there is deliberately no deny here. The *role* is un-narrowable because it is
    /// env-bootstrapped, but a deny row is a different lever and it still applies to an admin: see
    /// `a_deny_narrows_even_a_server_admin` below, which is the other half of this behaviour and
    /// must not be reconciled with this one by weakening either.
    #[sqlx::test]
    async fn a_server_admin_is_national_by_default(pool: sqlx::PgPool) {
        let user: String = sqlx::query_scalar(
            "insert into identity.users (full_name, display_name) values ('T', 'T') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into access.user_roles (user_id, role_name, source) values ($1, 'SERVER_ADMIN', 'system')",
        )
        .bind(&user)
        .execute(&pool)
        .await
        .unwrap();

        let scope = super::permission_scope(&pool, &user, "events.config.update")
            .await
            .unwrap();
        assert!(scope.is_national());
        assert!(scope.allows(Some("ZDC")));
        assert!(
            scope.allows(None),
            "national authority covers a resource with no owning ARTCC"
        );
    }

    /// An explicit deny narrows even a SERVER_ADMIN — and that is **pre-existing** behaviour, not
    /// something #543 introduced: the old view's anti-join ran over the whole candidate set,
    /// `server_admin_permissions` included, so a deny removed the permission from an admin too.
    ///
    /// Pinned because a resolver rewrite is exactly where it could be lost, and because
    /// "SERVER_ADMIN is untouchable" is an easy thing to assume. What is untouchable is the *role*:
    /// it stays env-bootstrapped. A deny row is a different lever, and it still works.
    #[sqlx::test]
    async fn a_deny_narrows_even_a_server_admin(pool: sqlx::PgPool) {
        let user: String = sqlx::query_scalar(
            "insert into identity.users (full_name, display_name) values ('T', 'T') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into access.user_roles (user_id, role_name, source) values ($1, 'SERVER_ADMIN', 'system')",
        )
        .bind(&user)
        .execute(&pool)
        .await
        .unwrap();
        crate::scope_test_support::deny_scoped(&pool, &user, "events.config.update", Some("ZDC"))
            .await;

        let scope = super::permission_scope(&pool, &user, "events.config.update")
            .await
            .unwrap();
        assert!(scope.is_national(), "still national overall");
        assert!(!scope.allows(Some("ZDC")), "but not at the denied ARTCC");
        assert!(scope.allows(Some("ZNY")), "and untouched elsewhere");

        // A national deny takes it away entirely, as it did before.
        crate::scope_test_support::deny_scoped(&pool, &user, "events.rate.update", None).await;
        assert!(
            super::permission_scope(&pool, &user, "events.rate.update")
                .await
                .unwrap()
                .is_empty()
        );
    }

    // ---- VATUSA/OIS#544: the seeded group defaults ----

    /// The domain rules migration 0094 seeded, as `web/src/lib/presets.ts` defines them. `None` means
    /// every permission in the catalogue.
    const SEEDED_ROLE_DOMAINS: &[(&str, Option<&[&str]>)] = &[
        ("VATUSA_STAFF", None),
        (
            "DCC_STAFF",
            Some(&["tmu", "flow", "events", "ace", "stats"]),
        ),
        ("EC", Some(&["tmu", "flow", "events", "ace", "stats"])),
        ("AEC", Some(&["tmu", "flow", "events", "ace", "stats"])),
        ("NTMO", Some(&["tmu", "flow", "stats"])),
        ("EVENTS_TEAM", Some(&["events", "ace"])),
        ("ACE", Some(&["ace"])),
    ];

    /// Grants a role holds that its preset's domains do **not** cover — the presets were never the
    /// whole truth, and this test found that out (#544).
    ///
    /// `NTMO` has carried `events.availability.update` since `0052_event_availability.sql:26`, added
    /// deliberately: *"NTMOs and DCC staff may respond by default"*. The NTMO preset grants
    /// `tmu/flow/stats` only, so aligning the seed to the preset would have quietly taken that
    /// capability away from every NTMO. It is kept, and named here so the next reader sees a decision
    /// rather than an anomaly. `DCC_STAFF` needs no entry: its domains already include `events`.
    const DOCUMENTED_EXTRAS: &[(&str, &[&str])] = &[("NTMO", &["events.availability.update"])];

    /// AC1 + AC2. Each seeded group must equal the live-catalogue expansion of its preset's domains.
    ///
    /// **This test is also the drift alarm.** Seeding turned a rule the presets evaluated against the
    /// *current* catalogue into a fixed row set, so a permission added later would not reach any group
    /// — silently, and most consequentially for `VATUSA_STAFF`, which is supposed to mean "everything".
    /// Adding permission 81 therefore fails here until someone decides which groups get it. That is the
    /// intended cost, and it matches how OIS already treats a new permission (marker + catalog + row).
    #[sqlx::test]
    async fn seeded_roles_match_the_preset_domains(pool: sqlx::PgPool) {
        let catalog: Vec<String> = sqlx::query_scalar("select name from access.permissions")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert!(!catalog.is_empty(), "the catalogue seed must have run");

        for (role, domains) in SEEDED_ROLE_DOMAINS {
            let mut expected: std::collections::BTreeSet<String> = match domains {
                None => catalog.iter().cloned().collect(),
                Some(allowed) => catalog
                    .iter()
                    .filter(|name| {
                        let domain = name.split('.').next().unwrap_or("");
                        allowed.contains(&domain)
                    })
                    .cloned()
                    .collect(),
            };
            for (extra_role, names) in DOCUMENTED_EXTRAS {
                if extra_role == role {
                    expected.extend(names.iter().map(|n| (*n).to_string()));
                }
            }

            let actual: std::collections::BTreeSet<String> = sqlx::query_scalar(
                "select permission_name from access.role_permissions where role_name = $1",
            )
            .bind(role)
            .fetch_all(&pool)
            .await
            .unwrap()
            .into_iter()
            .collect();

            assert_eq!(
                actual, expected,
                "{role}: seeded set has drifted from the preset's domains plus documented extras. If a role legitimately holds something outside its domains, add it to DOCUMENTED_EXTRAS with the reason. If you just added a \
                 permission, decide which groups should carry it and extend migration 0094."
            );
            assert!(!actual.is_empty(), "{role} must bundle something (#544)");
        }
    }

    /// The baseline every signed-in user gets, now held by the `USER` group rather than copied onto
    /// each user as five direct rows.
    #[sqlx::test]
    async fn the_user_group_carries_the_signed_in_baseline(pool: sqlx::PgPool) {
        let baseline: std::collections::BTreeSet<String> = sqlx::query_scalar(
            "select permission_name from access.role_permissions where role_name = 'USER'",
        )
        .fetch_all(&pool)
        .await
        .unwrap()
        .into_iter()
        .collect();

        for name in [
            "auth.profile.read",
            "auth.profile.update",
            "auth.sessions.delete",
            "access.self.read",
            "users.directory.read",
        ] {
            assert!(baseline.contains(name), "the USER group must carry {name}");
        }
    }

    /// A member of the group resolves to its whole set through the #543 resolver — the property that
    /// makes groups worth having: edit the group, every holder changes, no backfill.
    #[sqlx::test]
    async fn a_group_member_resolves_to_the_groups_permissions(pool: sqlx::PgPool) {
        let user: String = sqlx::query_scalar(
            "insert into identity.users (full_name, display_name) values ('T', 'T') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("insert into access.user_roles (user_id, role_name, source) values ($1, 'NTMO', 'manual')")
            .bind(&user)
            .execute(&pool)
            .await
            .unwrap();

        let effective = super::fetch_effective_permissions(&pool, &user)
            .await
            .unwrap();

        // NTMO is tmu/flow/stats, so a TMU permission is in.
        assert!(
            effective.keys().any(|n| n.starts_with("tmu.")),
            "an NTMO member holds the tmu domain"
        );
        // The events domain is not NTMO's, with one documented exception — see DOCUMENTED_EXTRAS.
        // Asserting on a specific broad permission rather than the whole `events.` prefix, so this
        // test says what it means instead of accidentally depending on that exception.
        assert!(
            !effective.contains_key("events.plan.read"),
            "NTMO does not bundle the events domain"
        );
        assert!(
            effective.contains_key("events.availability.update"),
            "except the availability grant 0052 gave it deliberately"
        );
        assert!(effective.values().all(|scope| scope.is_national()));
    }

    /// Migration 0094's cleanup of the redundant baseline rows.
    ///
    /// The migration ran against whatever rows existed at migration time and cannot see rows a test
    /// inserts afterwards, so this re-runs its `delete` against rows built here. It runs the statement
    /// **read out of the migration file** ([`migration_0094`]) rather than a copy typed into the test:
    /// a copy is what this test used to hold, and removing the `USER`-role guard from the real
    /// migration left it green, because it was proving a string in the test file (#544 review).
    ///
    /// What it proves is that the deletion is **narrow** — only the five names the `USER` group now
    /// supplies, only at national scope, only granted rows (never a deny), and only for users who
    /// actually hold the role, so it cannot strip access from someone the grant above missed.
    #[sqlx::test]
    async fn the_baseline_cleanup_removes_only_what_the_group_now_supplies(pool: sqlx::PgPool) {
        let in_group: String = sqlx::query_scalar(
            "insert into identity.users (full_name, display_name) values ('A', 'A') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let orphan: String = sqlx::query_scalar(
            "insert into identity.users (full_name, display_name) values ('B', 'B') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        // In the group, with a national *deny* on a baseline name. The cleanup removes redundant
        // grants; a deny is an admin's decision and must survive it.
        let denied: String = sqlx::query_scalar(
            "insert into identity.users (full_name, display_name) values ('C', 'C') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        // The pre-#544 world: direct baseline rows on both users, but only one is in the group.
        for user in [&in_group, &denied] {
            sqlx::query(
                "insert into access.user_roles (user_id, role_name, source) \
                 values ($1, 'USER', 'manual')",
            )
            .bind(user)
            .execute(&pool)
            .await
            .unwrap();
        }
        sqlx::query(
            "insert into access.user_permissions \
                 (user_id, permission_name, granted, artcc_id, source) \
             values ($1, 'users.directory.read', false, null, 'manual')",
        )
        .bind(&denied)
        .execute(&pool)
        .await
        .unwrap();
        for user in [&in_group, &orphan] {
            for name in [
                "auth.profile.read",
                "auth.profile.update",
                "auth.sessions.delete",
                "access.self.read",
                "users.directory.read",
            ] {
                crate::scope_test_support::grant(&pool, user, name, None).await;
            }
            // Two rows the cleanup must not touch: a non-baseline grant, and a scoped one.
            crate::scope_test_support::grant(&pool, user, "tmu.program.update", None).await;
            crate::scope_test_support::grant(&pool, user, "access.self.read", Some("ZDC")).await;
        }

        sqlx::query(&migration_0094("delete from access.user_permissions"))
            .execute(&pool)
            .await
            .unwrap();

        let remaining = |user: &str| {
            let pool = pool.clone();
            let user = user.to_string();
            async move {
                sqlx::query_scalar::<_, String>(
                    "select permission_name || coalesce(':' || artcc_id, '') \
                     from access.user_permissions where user_id = $1 order by 1",
                )
                .bind(user)
                .fetch_all(&pool)
                .await
                .unwrap()
            }
        };

        // In the group: the five national baseline rows are gone; the others survive.
        assert_eq!(
            remaining(&in_group).await,
            vec![
                "access.self.read:ZDC".to_string(),
                "tmu.program.update".to_string()
            ],
            "only the redundant national baseline rows should go"
        );

        // Not in the group: nothing is touched, because the group is not supplying it.
        assert_eq!(
            remaining(&orphan).await.len(),
            7,
            "a user the USER grant missed must keep their own rows"
        );

        // In the group, but denied: the deny is not a redundant grant, so it stays.
        assert_eq!(
            remaining(&denied).await,
            vec!["users.directory.read".to_string()],
            "the cleanup must never remove a deny"
        );
    }

    /// Migration 0094, from the same file the migrator embeds.
    const MIGRATION_0094: &str = include_str!("../../migrations/0094_seed_role_permissions.sql");

    /// 0094's statements in order. Comments are stripped *before* splitting on `;`, because four of
    /// the migration's comments contain one and a naive split would cut statements in half.
    fn migration_0094_statements() -> Vec<String> {
        MIGRATION_0094
            .lines()
            .map(|line| line.split("--").next().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n")
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// The one statement of 0094 that starts with `prefix` — so a test runs what production runs.
    fn migration_0094(prefix: &str) -> String {
        migration_0094_statements()
            .into_iter()
            .find(|statement| statement.starts_with(prefix))
            .unwrap_or_else(|| panic!("migration 0094 has no statement starting `{prefix}`"))
    }

    /// The pre-deploy audit for 0094, whose blocks the test below runs verbatim.
    const AUDIT_0094: &str = include_str!("../../audits/0094_role_seed_effect.sql");

    /// The text between `-- BEGIN {name}` and `-- END {name}` in the audit.
    fn audit_block(name: &str) -> &'static str {
        let begin = format!("-- BEGIN {name}");
        let end = format!("-- END {name}");
        let from = AUDIT_0094
            .find(&begin)
            .unwrap_or_else(|| panic!("audit has no `{begin}`"))
            + begin.len();
        let to = AUDIT_0094[from..]
            .find(&end)
            .unwrap_or_else(|| panic!("audit has no `{end}`"))
            + from;
        AUDIT_0094[from..to].trim()
    }

    /// `backend/audits/0094_role_seed_effect.sql` reports exactly whose access 0094 changes.
    ///
    /// #544's AC4 says no user's effective permissions change, verified against real users — and they
    /// do change for anyone holding EC, AEC, EVENTS_TEAM or VATUSA_STAFF whose grants an admin narrowed,
    /// because the editor stores an unticked box as an absent row rather than a deny, and those roles
    /// granted nothing until 0094. The audit is how the owner sees that population before deploying.
    ///
    /// This runs the audit's own snapshot and diff blocks inside a rolled-back transaction, exactly as
    /// `psql -f` would, with 0094's statements in between. The pre-0094 world is rebuilt by emptying
    /// the four roles 0094 seeds from nothing — they held no `role_permissions` before it (`0060`).
    #[sqlx::test]
    async fn the_0094_audit_reports_exactly_who_the_seed_would_change(pool: sqlx::PgPool) {
        use sqlx::Row;

        sqlx::query(
            "delete from access.role_permissions \
             where role_name in ('VATUSA_STAFF', 'EC', 'AEC', 'EVENTS_TEAM')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let user = |name: &'static str| {
            let pool = pool.clone();
            async move {
                sqlx::query_scalar::<_, String>(
                    "insert into identity.users (full_name, display_name) values ($1, $1) returning id",
                )
                .bind(name)
                .fetch_one(&pool)
                .await
                .unwrap()
            }
        };
        let role = |user: String, role: &'static str, artcc: Option<&'static str>| {
            let pool = pool.clone();
            async move {
                sqlx::query(
                    "insert into access.user_roles (user_id, role_name, artcc_id, source) \
                     values ($1, $2, $3, 'manual')",
                )
                .bind(user)
                .bind(role)
                .bind(artcc)
                .execute(&pool)
                .await
                .unwrap();
            }
        };

        // Every pre-#544 account carries the login baseline as five direct national rows, and holds no
        // `USER` role — nothing granted one until 0094. Modelled here so the test users look like real
        // ones; without it they would "gain" the baseline itself, which no real user does.
        let login_baseline = |user: String| {
            let pool = pool.clone();
            async move {
                for name in [
                    "auth.profile.read",
                    "auth.profile.update",
                    "auth.sessions.delete",
                    "access.self.read",
                    "users.directory.read",
                ] {
                    crate::scope_test_support::grant(&pool, &user, name, None).await;
                }
            }
        };

        // An account with no role at all — the bulk of the real population.
        let plain = user("plain").await;
        login_baseline(plain.clone()).await;

        // Exactly what the editor's save leaves behind for a narrowed holder: the role, plus granted
        // rows for the ticked boxes only.
        let staff = user("narrowed-staff").await;
        login_baseline(staff.clone()).await;
        role(staff.clone(), "VATUSA_STAFF", None).await;
        crate::scope_test_support::grant(&pool, &staff, "stats.data.read", None).await;

        let ec = user("narrowed-ec").await;
        login_baseline(ec.clone()).await;
        role(ec.clone(), "EC", Some("ZDC")).await;
        crate::scope_test_support::grant(&pool, &ec, "events.plan.read", Some("ZDC")).await;

        // An EC who already holds the whole seeded set directly: the seed adds nothing for them.
        let full = user("full-ec").await;
        login_baseline(full.clone()).await;
        role(full.clone(), "EC", Some("ZDC")).await;
        let operational: Vec<String> = sqlx::query_scalar(
            "select name from access.permissions \
             where split_part(name, '.', 1) in ('tmu', 'flow', 'events', 'ace', 'stats')",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        for name in &operational {
            crate::scope_test_support::grant(&pool, &full, name, Some("ZDC")).await;
        }

        // What the `USER` group carries beyond the old five-row login baseline. Granting the group to
        // everyone hands every account these, which is a real change AC4 covers: `0047` put
        // `ace.requests.create` on `USER`, but nothing ever granted the role, so it sat dormant.
        let universal: std::collections::BTreeSet<String> = sqlx::query_scalar(
            "select permission_name from access.role_permissions where role_name = 'USER' \
             and permission_name not in ('auth.profile.read', 'auth.profile.update', \
                 'auth.sessions.delete', 'access.self.read', 'users.directory.read')",
        )
        .fetch_all(&pool)
        .await
        .unwrap()
        .into_iter()
        .collect();

        let mut tx = pool.begin().await.unwrap();
        // 0094 predates `source` (0098), so its inserts do not name it — and 0098 drops the column's
        // backfill default on purpose, so replaying 0094 against a fully-migrated schema violates
        // `not null`. Restore the default for the replay only: the rollback below undoes it along with
        // everything else, and 0094's statements stay byte-identical, which is what makes this test
        // run what production ran.
        for table in ["access.user_roles", "access.user_permissions"] {
            sqlx::query(&format!(
                "alter table {table} alter column source set default 'manual'"
            ))
            .execute(&mut *tx)
            .await
            .unwrap();
        }
        sqlx::query(audit_block("SNAPSHOT"))
            .execute(&mut *tx)
            .await
            .unwrap();
        for statement in migration_0094_statements() {
            sqlx::query(&statement).execute(&mut *tx).await.unwrap();
        }
        let rows = sqlx::query(audit_block("DIFF"))
            .fetch_all(&mut *tx)
            .await
            .unwrap();
        tx.rollback().await.unwrap();

        let changes: std::collections::BTreeSet<(String, String, String, String)> = rows
            .iter()
            .map(|r| {
                (
                    r.get::<String, _>("change"),
                    r.get::<String, _>("user_id"),
                    r.get::<String, _>("permission_name"),
                    r.get::<String, _>("scope"),
                )
            })
            .collect();
        let has = |change: &str, who: &str, permission: &str, scope: &str| {
            changes.contains(&(
                change.to_string(),
                who.to_string(),
                permission.to_string(),
                scope.to_string(),
            ))
        };

        assert!(
            has("gained", &staff, "access.users.update", "NATIONAL"),
            "the narrowed staff member regains the right to edit anyone's access"
        );
        assert!(
            has("gained", &ec, "tmu.ntml.create", "ZDC"),
            "the narrowed EC regains TMU at ZDC"
        );
        let gains_of = |who: &str| -> std::collections::BTreeSet<String> {
            changes
                .iter()
                .filter(|(change, user, _, scope)| {
                    change == "gained" && user == who && scope == "NATIONAL"
                })
                .map(|(_, _, permission, _)| permission.clone())
                .collect()
        };
        assert_eq!(
            gains_of(&plain),
            universal,
            "an account with no role gains exactly what USER carries beyond the login baseline"
        );
        assert!(
            changes.iter().filter(|(_, who, _, _)| who == &full).all(
                |(change, _, permission, scope)| change == "gained"
                    && scope == "NATIONAL"
                    && universal.contains(permission)
            ),
            "an EC who already held the whole set gains only what every account gains: {changes:?}"
        );
        assert!(
            !has("gained", &ec, "tmu.ntml.create", "NATIONAL"),
            "and the EC's regained access stays at ZDC rather than going national"
        );
        assert!(
            !changes.iter().any(|(change, _, _, _)| change == "lost"),
            "no one may lose anything: {changes:?}"
        );
    }

    /// A facility-scoped membership narrows the whole group, which is the design's point: scope lives
    /// on the membership row, not on the bundle, so one `EC` group serves every ARTCC.
    #[sqlx::test]
    async fn a_facility_scoped_membership_narrows_the_whole_group(pool: sqlx::PgPool) {
        let user: String = sqlx::query_scalar(
            "insert into identity.users (full_name, display_name) values ('T', 'T') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into access.user_roles (user_id, role_name, artcc_id, source) values ($1, 'EC', 'ZDC', 'manual')",
        )
        .bind(&user)
        .execute(&pool)
        .await
        .unwrap();

        let effective = super::fetch_effective_permissions(&pool, &user)
            .await
            .unwrap();
        let scope = effective
            .get("flow.route.update")
            .expect("EC bundles the flow domain");
        assert!(scope.allows(Some("ZDC")));
        assert!(
            !scope.allows(Some("ZNY")),
            "a ZDC membership grants nothing at ZNY"
        );
    }
}
