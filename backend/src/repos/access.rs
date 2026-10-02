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

/// Resolve an `ois_pat_…` bearer token to its API key, if active/unrevoked/unexpired and the owner
/// is still an active user. Updates `last_used_at`/`last_used_ip` on a hit. The key's *authority* is
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
        sqlx::query(
            "update access.api_keys set last_used_at = now(), \
             last_used_ip = coalesce($2::inet, last_used_ip) where id = $1",
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::PermissionScope;

    fn facilities(ids: &[&str]) -> PermissionScope {
        PermissionScope::Facilities(ids.iter().map(|s| s.to_string()).collect())
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
                "insert into access.user_roles (user_id, role_name, artcc_id) \
                 values ($1, 'DENY_TEST', $2)",
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
            "insert into access.user_roles (user_id, role_name, artcc_id) \
             values ($1, 'SCOPE_TEST', 'ZDC')",
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

    /// SERVER_ADMIN is national whatever else is on the row, and a deny must not narrow it —
    /// it stays env-bootstrapped and un-narrowable.
    #[sqlx::test]
    async fn a_server_admin_is_national_and_cannot_be_denied_away(pool: sqlx::PgPool) {
        let user: String = sqlx::query_scalar(
            "insert into identity.users (full_name, display_name) values ('T', 'T') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into access.user_roles (user_id, role_name) values ($1, 'SERVER_ADMIN')",
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
            "insert into access.user_roles (user_id, role_name) values ($1, 'SERVER_ADMIN')",
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
}
