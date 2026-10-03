//! The access-editor backend: read the catalog, read a user's access, and save
//! roles + permission grants with a required reason (audited). Ported from osmium's
//! admin access handlers and extended with per-ARTCC scope: grants can be national
//! (`artcc_id = null`) or scoped to a facility.

use std::collections::{BTreeMap, BTreeSet};

use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    http::HeaderMap,
};
use serde::Deserialize;

use crate::{
    auth::{
        acl::{
            fetch_user_access, is_server_admin, normalize_permission_tree,
            permission_tree_from_names,
        },
        context::CurrentUser,
        permissions::{AccessCatalogRead, AccessSelfRead, AccessUsersRead, AccessUsersUpdate},
        require_permission::RequirePermission,
    },
    errors::ApiError,
    models::{
        AccessCatalogBody, AdminUserPage, ScopeAccess, SelfAccessBody, UpdateUserAccessRequest,
        UserAccessBody,
    },
    repos::{access as access_repo, audit as audit_repo, org as org_repo, users as user_repo},
    state::AppState,
};

#[derive(Deserialize)]
pub struct UserListQuery {
    /// Name substring or CID prefix; empty lists everyone.
    q: Option<String>,
    page: Option<i64>,
    page_size: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/users",
    tag = "access",
    params(
        ("q" = Option<String>, Query, description = "Name substring or CID prefix"),
        ("page" = Option<i64>, Query, description = "1-based page (default 1)"),
        ("page_size" = Option<i64>, Query, description = "Rows per page (default 25, max 100)")
    ),
    responses((status = 200, body = AdminUserPage), (status = 401))
)]
pub async fn list_users(
    State(state): State<AppState>,
    _permission: RequirePermission<AccessUsersRead>,
    Query(query): Query<UserListQuery>,
) -> Result<Json<AdminUserPage>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let q = query.q.unwrap_or_default().trim().to_string();
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(25).clamp(1, 100);
    let offset = (page - 1) * page_size;

    let items = user_repo::list_users(pool, &q, page_size, offset).await?;
    let total = user_repo::count_users(pool, &q).await?;
    Ok(Json(AdminUserPage {
        items,
        total,
        page,
        page_size,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/access/catalog",
    tag = "access",
    responses((status = 200, body = AccessCatalogBody), (status = 401))
)]
pub async fn get_access_catalog(
    State(state): State<AppState>,
    _permission: RequirePermission<AccessCatalogRead>,
) -> Result<Json<AccessCatalogBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let permission_names = access_repo::fetch_access_catalog_names(pool).await?;
    Ok(Json(AccessCatalogBody {
        roles: access_repo::ASSIGNABLE_USER_ROLES
            .iter()
            .map(|role| role.to_string())
            .collect(),
        permissions: permission_tree_from_names(&permission_names)?,
        facilities: org_repo::list_facilities(pool).await?,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/access/self",
    tag = "access",
    responses((status = 200, body = SelfAccessBody), (status = 401))
)]
pub async fn get_self_access(
    State(state): State<AppState>,
    _permission: RequirePermission<AccessSelfRead>,
    Extension(current_user): Extension<Option<CurrentUser>>,
) -> Result<Json<SelfAccessBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let (roles, permissions) = fetch_user_access(state.db.as_ref(), &user.id).await?;
    Ok(Json(SelfAccessBody {
        server_admin: is_server_admin(&roles),
        role_names: roles,
        permissions: crate::auth::acl::permission_tree_from_paths(&permissions),
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/users/{cid}/access",
    tag = "access",
    params(("cid" = i64, Path, description = "VATSIM CID")),
    responses((status = 200, body = UserAccessBody), (status = 401), (status = 404))
)]
pub async fn get_user_access(
    State(state): State<AppState>,
    _permission: RequirePermission<AccessUsersRead>,
    Path(cid): Path<i64>,
) -> Result<Json<UserAccessBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let target = access_repo::find_current_user_by_cid(pool, cid)
        .await?
        .ok_or(ApiError::NotFound)?;
    let grants = access_repo::fetch_user_direct_grants(pool, &target.id).await?;
    let roles = access_repo::fetch_user_role_grants(pool, &target.id).await?;
    let mut body = build_user_access_body(&target.id, target.cid, grants, roles)?;
    fill_server_admin_permissions(pool, &mut body).await?;
    Ok(Json(body))
}

/// Server admins implicitly hold every permission (via the effective-permissions view).
/// Surface that in the editor by showing the full catalog at the national scope; the UI
/// renders the permission tree read-only (roles remain editable).
async fn fill_server_admin_permissions(
    pool: &sqlx::PgPool,
    body: &mut UserAccessBody,
) -> Result<(), ApiError> {
    if body.server_admin {
        let all = access_repo::fetch_access_catalog_names(pool).await?;
        let tree = permission_tree_from_names(&all)?;
        if let Some(national) = body.scopes.iter_mut().find(|s| s.artcc_id.is_none()) {
            national.permissions = tree;
        }
    }
    Ok(())
}

#[utoipa::path(
    post,
    path = "/api/v1/admin/users/{cid}/access",
    tag = "access",
    params(("cid" = i64, Path, description = "VATSIM CID")),
    request_body = UpdateUserAccessRequest,
    responses((status = 200, body = UserAccessBody), (status = 400), (status = 401), (status = 403), (status = 404))
)]
pub async fn update_user_access(
    State(state): State<AppState>,
    _permission: RequirePermission<AccessUsersUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(cid): Path<i64>,
    headers: HeaderMap,
    Json(payload): Json<UpdateUserAccessRequest>,
) -> Result<Json<UserAccessBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    let reason = payload.reason.trim();
    if reason.is_empty() {
        return Err(ApiError::BadRequest);
    }

    let catalog: BTreeSet<String> = access_repo::fetch_access_catalog_names(pool)
        .await?
        .into_iter()
        .collect();
    let facility_ids: BTreeSet<String> = org_repo::list_facilities(pool)
        .await?
        .into_iter()
        .map(|facility| facility.id)
        .collect();

    // Validate + normalize every scope up front.
    let mut norm_scopes: Vec<NormScope> = Vec::with_capacity(payload.scopes.len());
    for scope in &payload.scopes {
        let artcc = scope
            .artcc_id
            .as_deref()
            .map(|value| value.trim().to_ascii_uppercase())
            .filter(|value| !value.is_empty());
        if let Some(artcc_id) = &artcc
            && !facility_ids.contains(artcc_id)
        {
            return Err(ApiError::BadRequest);
        }

        let names = scope_permission_names(&scope.permissions)?;
        if let Some(unknown) = names.iter().find(|name| !catalog.contains(*name)) {
            tracing::warn!(
                permission = unknown.as_str(),
                "unknown permission in access save"
            );
            return Err(ApiError::BadRequest);
        }

        if let Some(role_names) = scope.role_names.as_ref() {
            for role_name in role_names {
                if !access_repo::ASSIGNABLE_USER_ROLES.contains(&role_name.as_str()) {
                    return Err(ApiError::BadRequest);
                }
            }
        }

        norm_scopes.push(NormScope {
            artcc,
            names,
            roles: scope.role_names.clone(),
        });
    }

    let target_user_id = access_repo::find_user_id_by_cid(pool, cid)
        .await?
        .ok_or(ApiError::NotFound)?;
    let target = access_repo::find_current_user_by_cid(pool, cid)
        .await?
        .ok_or(ApiError::NotFound)?;

    let before_grants = access_repo::fetch_user_direct_grants(pool, &target_user_id).await?;
    let before_roles = access_repo::fetch_user_role_grants(pool, &target_user_id).await?;
    let before_body = build_user_access_body(
        &target_user_id,
        target.cid,
        before_grants.clone(),
        before_roles.clone(),
    )?;

    // A server admin's *permissions* are not editable — they hold everything implicitly
    // via the effective view. Their roles remain editable, so we still process role
    // changes but skip any direct-permission changes for a server-admin target.
    let target_is_server_admin = before_body.server_admin;

    enforce_actor_scope(&state, user, &norm_scopes, &before_grants, &before_roles).await?;

    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    for scope in &norm_scopes {
        if !target_is_server_admin {
            access_repo::replace_user_permissions_scoped(
                &mut tx,
                &target_user_id,
                scope.artcc.as_deref(),
                &scope.names,
            )
            .await?;
        }
        if let Some(role_names) = scope.roles.as_ref() {
            for role_name in access_repo::ASSIGNABLE_USER_ROLES {
                let held = role_names.iter().any(|r| r == role_name);
                access_repo::set_user_role_manual_scoped(
                    &mut tx,
                    &target_user_id,
                    role_name,
                    held,
                    scope.artcc.as_deref(),
                )
                .await?;
            }
        }
    }
    tx.commit().await.map_err(|_| ApiError::Internal)?;

    let after_grants = access_repo::fetch_user_direct_grants(pool, &target_user_id).await?;
    let after_roles = access_repo::fetch_user_role_grants(pool, &target_user_id).await?;
    let mut response =
        build_user_access_body(&target_user_id, target.cid, after_grants, after_roles)?;
    fill_server_admin_permissions(pool, &mut response).await?;

    let actor_id = audit_repo::fetch_user_actor_id(pool, &user.id).await?;
    audit_repo::record_audit(
        pool,
        audit_repo::AuditEntry {
            actor_id,
            action: "UPDATE".to_string(),
            resource_type: "USER_ACCESS".to_string(),
            resource_id: Some(target_user_id.clone()),
            artcc_id: None,
            reason: Some(reason.to_string()),
            before_state: serde_json::to_value(&before_body).ok(),
            after_state: serde_json::to_value(&response).ok(),
            ip_address: audit_repo::client_ip(&headers),
        },
    )
    .await?;

    // Tell connected clients their access may have changed. Deliberately fired for any save rather
    // than only for additions: this is an invalidation nudge, and deciding whether *you* gained
    // anything is the client's job — it is the only side that knows what you held before.
    state.publish(crate::realtime::topic::ACCESS_GRANTED);

    Ok(Json(response))
}

/// A validated, normalized scope from the save payload.
struct NormScope {
    artcc: Option<String>,
    names: Vec<String>,
    /// `None` = leave this scope's roles untouched; `Some` = replace them.
    roles: Option<Vec<String>>,
}

/// Empty object `{}` means "clear this scope's direct grants"; anything else must be a
/// valid permission tree.
fn scope_permission_names(tree: &serde_json::Value) -> Result<Vec<String>, ApiError> {
    if tree.as_object().is_some_and(|object| object.is_empty()) {
        return Ok(Vec::new());
    }
    normalize_permission_tree(tree).ok_or(ApiError::BadRequest)
}

/// Self-scope guard: a non-SERVER_ADMIN actor may only add/remove direct grants they themselves
/// hold **at that scope**, and roles they hold, and only within the scopes they are editing. Diffs
/// against the target's current grants so untouched scopes/permissions aren't disturbed.
///
/// Denies need no modelling here because the editor cannot change them: the save replaces grants only
/// (`replace_user_permissions_scoped`). The roles half is still name-only — the same scope-blindness,
/// tracked separately (#559 follow-up).
async fn enforce_actor_scope(
    state: &AppState,
    actor: &CurrentUser,
    norm_scopes: &[NormScope],
    before_grants: &[(Option<String>, String)],
    before_roles: &[(Option<String>, String)],
) -> Result<(), ApiError> {
    let (actor_roles, _) = fetch_user_access(state.db.as_ref(), &actor.id).await?;
    if is_server_admin(&actor_roles) {
        return Ok(());
    }
    // The actor's authority **per permission, per scope, after denies** — from the unified resolver
    // (#543), not from permission names (#559). Names alone made a ZDC-scoped editor read as national,
    // and a permission they had been denied still read as theirs to delegate.
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let actor_authority = access_repo::fetch_effective_permissions(pool, &actor.id).await?;

    // Permissions: only scopes present in the payload are changed.
    let payload_scopes: BTreeSet<Option<String>> = norm_scopes
        .iter()
        .map(|scope| scope.artcc.clone())
        .collect();
    let requested_perms: BTreeSet<(Option<String>, String)> = norm_scopes
        .iter()
        .flat_map(|scope| {
            scope
                .names
                .iter()
                .map(move |name| (scope.artcc.clone(), name.clone()))
        })
        .collect();
    let existing_perms: BTreeSet<(Option<String>, String)> = before_grants
        .iter()
        .filter(|(artcc, _)| payload_scopes.contains(artcc))
        .cloned()
        .collect();
    // Adding *or* removing a grant at a scope needs the actor to hold it there. `allows(None)` — a
    // national grant — needs **unrestricted** national holding, so someone holding P "nationally except
    // ZNY" cannot hand out a national P and leak ZNY through it.
    //
    // 401 when the actor holds the permission nowhere, 403 when they hold it but not at this scope: the
    // codebase's split between "absent" and "wrong facility".
    for (artcc, name) in requested_perms.symmetric_difference(&existing_perms) {
        match actor_authority.get(name) {
            None => return Err(ApiError::Unauthorized),
            Some(scope) if scope.is_empty() => return Err(ApiError::Unauthorized),
            Some(scope) if !scope.allows(artcc.as_deref()) => return Err(ApiError::Forbidden),
            Some(_) => {}
        }
    }

    // Roles: only scopes whose `roles` is present are changed.
    let role_scopes: BTreeSet<Option<String>> = norm_scopes
        .iter()
        .filter(|scope| scope.roles.is_some())
        .map(|scope| scope.artcc.clone())
        .collect();
    let requested_roles: BTreeSet<(Option<String>, String)> = norm_scopes
        .iter()
        .filter_map(|scope| {
            scope
                .roles
                .as_ref()
                .map(|roles| (scope.artcc.clone(), roles))
        })
        .flat_map(|(artcc, roles)| roles.iter().map(move |role| (artcc.clone(), role.clone())))
        .collect();
    let existing_roles: BTreeSet<(Option<String>, String)> = before_roles
        .iter()
        .filter(|(artcc, role)| {
            role_scopes.contains(artcc)
                && access_repo::ASSIGNABLE_USER_ROLES.contains(&role.as_str())
        })
        .cloned()
        .collect();
    for (_, role) in requested_roles.symmetric_difference(&existing_roles) {
        if !actor_roles.contains(role) {
            return Err(ApiError::Forbidden);
        }
    }

    Ok(())
}

/// Groups direct grants + role assignments into per-scope `ScopeAccess` (national first).
fn build_user_access_body(
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

#[cfg(test)]
mod escalation_guard_tests {
    //! VATUSA/OIS#559: `enforce_actor_scope` — the access editor's no-escalation gate — had no tests at
    //! all. These drive the real route, `POST /api/v1/admin/users/{cid}/access`, adversarially: each one
    //! is an attempt to grant what the actor does not hold, at a scope they do not hold it at.

    use std::collections::HashMap;

    use axum::http;
    use serde_json::{Value, json};
    use sqlx::PgPool;

    use crate::scope_test_support::{
        deny_scoped, grant, seed_user, send, session_cookie, test_state,
    };

    /// The permission being fought over. A real catalog entry, so validation passes and the guard is
    /// what decides.
    const P: &str = "tmu.program.update";
    const TARGET_CID: i64 = 9_000_559;

    fn tree() -> Value {
        json!({"tmu": {"program": ["update"]}})
    }

    struct World {
        state: crate::state::AppState,
        pool: PgPool,
        actor: String,
        cookie: String,
        target: String,
    }

    /// An actor who may use the editor at all (`access.users.update`), and a target to edit.
    async fn world(pool: PgPool) -> World {
        let state = test_state(pool.clone(), HashMap::new());
        let actor = seed_user(&pool).await;
        grant(&pool, &actor, "access.users.update", None).await;
        let cookie = session_cookie(&pool, &actor).await;
        let target: String = sqlx::query_scalar(
            "insert into identity.users (cid, full_name, display_name) \
             values ($1, 'Target', 'Target') returning id",
        )
        .bind(TARGET_CID)
        .fetch_one(&pool)
        .await
        .unwrap();
        World {
            state,
            pool,
            actor,
            cookie,
            target,
        }
    }

    /// Save one scope of the target's access: `artcc = None` is national.
    async fn save(w: &World, artcc: Option<&str>, permissions: Value) -> http::StatusCode {
        send(
            &w.state,
            http::Method::POST,
            &format!("/api/v1/admin/users/{TARGET_CID}/access"),
            &w.cookie,
            Some(json!({
                "reason": "test",
                "scopes": [{"artcc_id": artcc, "permissions": permissions}],
            })),
        )
        .await
    }

    /// The target's direct rows for `P`, as `scope:grant|deny`.
    async fn rows(w: &World) -> Vec<String> {
        sqlx::query_scalar(
            "select coalesce(artcc_id, 'national') || ':' || \
                    case when granted then 'grant' else 'deny' end \
             from access.user_permissions where user_id = $1 and permission_name = $2 order by 1",
        )
        .bind(&w.target)
        .bind(P)
        .fetch_all(&w.pool)
        .await
        .unwrap()
    }

    // ---- AC1: scope --------------------------------------------------------------------------------

    /// The escalation itself. Names alone made a ZDC-scoped editor read as national.
    #[sqlx::test]
    async fn a_zdc_editor_cannot_grant_at_another_artcc(pool: PgPool) {
        let w = world(pool).await;
        grant(&w.pool, &w.actor, P, Some("ZDC")).await;

        assert_eq!(
            save(&w, Some("ZNY"), tree()).await,
            http::StatusCode::FORBIDDEN
        );
        assert!(rows(&w).await.is_empty(), "a refused save writes nothing");
    }

    #[sqlx::test]
    async fn a_zdc_editor_cannot_grant_nationally(pool: PgPool) {
        let w = world(pool).await;
        grant(&w.pool, &w.actor, P, Some("ZDC")).await;

        assert_eq!(save(&w, None, tree()).await, http::StatusCode::FORBIDDEN);
        assert!(rows(&w).await.is_empty());
    }

    /// The positive control: the gate narrows, it does not disable. Without this, a guard that refused
    /// everything would pass the two tests above.
    #[sqlx::test]
    async fn a_zdc_editor_can_grant_at_zdc(pool: PgPool) {
        let w = world(pool).await;
        grant(&w.pool, &w.actor, P, Some("ZDC")).await;

        assert_eq!(save(&w, Some("ZDC"), tree()).await, http::StatusCode::OK);
        assert_eq!(rows(&w).await, ["ZDC:grant"]);
    }

    /// The diff is symmetric, so *taking* a grant away needs the same authority as giving it.
    #[sqlx::test]
    async fn a_zdc_editor_cannot_revoke_at_another_artcc(pool: PgPool) {
        let w = world(pool).await;
        grant(&w.pool, &w.actor, P, Some("ZDC")).await;
        grant(&w.pool, &w.target, P, Some("ZNY")).await;

        assert_eq!(
            save(&w, Some("ZNY"), json!({})).await,
            http::StatusCode::FORBIDDEN
        );
        assert_eq!(
            rows(&w).await,
            ["ZNY:grant"],
            "the ZNY grant survives the refused save"
        );
    }

    // ---- AC2: denies -------------------------------------------------------------------------------

    /// A ZDC grant plus a **national** deny resolves to holding P nowhere — the deny wins. By name the
    /// actor still "has" P, which is exactly what the old guard checked. 401, the "absent" answer.
    /// (A grant and a deny at the *same* scope can't coexist — the unique index forbids it.)
    #[sqlx::test]
    async fn an_actor_denied_a_permission_cannot_grant_it(pool: PgPool) {
        let w = world(pool).await;
        grant(&w.pool, &w.actor, P, Some("ZDC")).await;
        deny_scoped(&w.pool, &w.actor, P, None).await;

        assert_eq!(
            save(&w, Some("ZDC"), tree()).await,
            http::StatusCode::UNAUTHORIZED
        );
        assert!(rows(&w).await.is_empty());
    }

    /// National except ZNY: a national grant to someone else would hand them ZNY, which the actor does
    /// not hold. So it needs **unrestricted** national holding — while ZDC is fine.
    #[sqlx::test]
    async fn national_except_one_artcc_cannot_grant_nationally(pool: PgPool) {
        let w = world(pool).await;
        grant(&w.pool, &w.actor, P, None).await;
        deny_scoped(&w.pool, &w.actor, P, Some("ZNY")).await;

        assert_eq!(save(&w, None, tree()).await, http::StatusCode::FORBIDDEN);
        assert_eq!(
            save(&w, Some("ZNY"), tree()).await,
            http::StatusCode::FORBIDDEN
        );
        assert_eq!(save(&w, Some("ZDC"), tree()).await, http::StatusCode::OK);
        assert_eq!(rows(&w).await, ["ZDC:grant"]);
    }

    // ---- AC3: untouched scopes ---------------------------------------------------------------------

    /// A ZDC-only save must leave the target's ZNY grant alone, even though the actor could never have
    /// touched ZNY. The guard only weighs what the save changes.
    #[sqlx::test]
    async fn a_save_leaves_scopes_it_does_not_name_alone(pool: PgPool) {
        let w = world(pool).await;
        grant(&w.pool, &w.actor, P, Some("ZDC")).await;
        grant(&w.pool, &w.target, P, Some("ZNY")).await;

        assert_eq!(save(&w, Some("ZDC"), tree()).await, http::StatusCode::OK);
        assert_eq!(rows(&w).await, ["ZDC:grant", "ZNY:grant"]);
    }

    // ---- AC4: SERVER_ADMIN -------------------------------------------------------------------------

    #[sqlx::test]
    async fn a_server_admin_bypasses_the_guard(pool: PgPool) {
        let w = world(pool).await;
        sqlx::query(
            "insert into access.user_roles (user_id, role_name) values ($1, 'SERVER_ADMIN')",
        )
        .bind(&w.actor)
        .execute(&w.pool)
        .await
        .unwrap();

        assert_eq!(save(&w, None, tree()).await, http::StatusCode::OK);
        assert_eq!(rows(&w).await, ["national:grant"]);
    }

    // ---- the save no longer strips denies ----------------------------------------------------------

    /// An unchanged save used to delete the target's deny in that scope — widening their access with no
    /// guard involved, because the editor cannot express a deny and the diff never saw one.
    #[sqlx::test]
    async fn an_unchanged_save_keeps_the_targets_deny(pool: PgPool) {
        let w = world(pool).await;
        grant(&w.pool, &w.actor, P, None).await;
        grant(&w.pool, &w.actor, "tmu.ntml.update", None).await;
        deny_scoped(&w.pool, &w.target, P, Some("ZDC")).await;

        // Saves ZDC with a different permission; P is not mentioned.
        let other = json!({"tmu": {"ntml": ["update"]}});
        assert_eq!(save(&w, Some("ZDC"), other).await, http::StatusCode::OK);

        assert_eq!(rows(&w).await, ["ZDC:deny"], "the deny survives the save");
    }

    /// An explicit grant over a deny replaces it — the editor deliberately granting — rather than
    /// failing on the unique index. Gated like any grant: the actor holds P at ZDC.
    #[sqlx::test]
    async fn an_explicit_grant_over_a_deny_replaces_it(pool: PgPool) {
        let w = world(pool).await;
        grant(&w.pool, &w.actor, P, Some("ZDC")).await;
        deny_scoped(&w.pool, &w.target, P, Some("ZDC")).await;

        assert_eq!(save(&w, Some("ZDC"), tree()).await, http::StatusCode::OK);
        assert_eq!(rows(&w).await, ["ZDC:grant"]);
    }

    /// ...and refused when the actor could not grant it there anyway, leaving the deny in place.
    #[sqlx::test]
    async fn a_grant_over_a_deny_is_still_gated(pool: PgPool) {
        let w = world(pool).await;
        grant(&w.pool, &w.actor, P, Some("ZNY")).await;
        deny_scoped(&w.pool, &w.target, P, Some("ZDC")).await;

        assert_eq!(
            save(&w, Some("ZDC"), tree()).await,
            http::StatusCode::FORBIDDEN
        );
        assert_eq!(rows(&w).await, ["ZDC:deny"]);
    }
}
