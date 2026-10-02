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
        permissions::{
            AccessCatalogRead, AccessGroupsRead, AccessGroupsUpdate, AccessSelfRead,
            AccessUsersRead, AccessUsersUpdate,
        },
        require_permission::RequirePermission,
    },
    errors::ApiError,
    models::{
        AccessCatalogBody, AdminUserPage, CreateGroupRequest, GroupBody, ScopeAccess,
        SelfAccessBody, UpdateGroupRequest, UpdateUserAccessRequest, UserAccessBody,
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
        // Read from the catalogue, not a constant: since #545 an admin can create a group, and it
        // must be assignable at once rather than after a deploy.
        roles: access_repo::fetch_assignable_role_names(pool).await?,
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
    // Derived rather than the old constant, so a group created through the group editor is
    // assignable here without a deploy (#545).
    let assignable_roles: BTreeSet<String> = access_repo::fetch_assignable_role_names(pool)
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
                if !assignable_roles.contains(role_name) {
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

    enforce_actor_scope(
        &state,
        user,
        &norm_scopes,
        &before_grants,
        &before_roles,
        &assignable_roles,
    )
    .await?;

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
            for role_name in &assignable_roles {
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

/// Self-scope guard: a non-SERVER_ADMIN actor may only add/remove direct grants and
/// roles they themselves hold, and only within the scopes they are editing. Diffs
/// against the target's current grants so untouched scopes/permissions aren't disturbed.
async fn enforce_actor_scope(
    state: &AppState,
    actor: &CurrentUser,
    norm_scopes: &[NormScope],
    before_grants: &[(Option<String>, String)],
    before_roles: &[(Option<String>, String)],
    assignable_roles: &BTreeSet<String>,
) -> Result<(), ApiError> {
    let (actor_roles, actor_permissions) = fetch_user_access(state.db.as_ref(), &actor.id).await?;
    if is_server_admin(&actor_roles) {
        return Ok(());
    }
    let actor_perm_names: BTreeSet<String> = actor_permissions
        .iter()
        .map(|path| path.as_db_value())
        .collect();

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
    for (_, name) in requested_perms.symmetric_difference(&existing_perms) {
        if !actor_perm_names.contains(name) {
            return Err(ApiError::Unauthorized);
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
        .filter(|(artcc, role)| role_scopes.contains(artcc) && assignable_roles.contains(role))
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

// ---- Group (role) management — VATUSA/OIS#545 ----

/// The no-escalation gate for group edits.
///
/// **This is the heart of #545.** Group editing is a privilege-escalation path that did not exist
/// while `role_permissions` was migration-only: anyone who can edit a group can grant themselves
/// whatever they put in it, then hold that group.
///
/// Deliberately **not** a copy of [`enforce_actor_scope`], which this issue suggested mirroring.
/// That guard reads `fetch_user_direct_grants` (filters `granted = true`, so deny-blind) and
/// `fetch_user_access` (names with no `artcc_id`, so scope-blind) — see #559. Mirroring it would carry
/// both faults into the higher-risk path, so the actor's authority comes from
/// `fetch_effective_permissions` (#543), the one resolver that honours scope and deny together.
///
/// What it copies from `enforce_actor_scope` is the part that guard gets right: the diff is a
/// **symmetric difference**, so *removing* a permission from a group also requires holding it. You
/// cannot strip authority you do not have any more than you can grant it.
async fn enforce_group_scope(
    state: &AppState,
    actor: &CurrentUser,
    role_name: &str,
    before: &[String],
    after: &[String],
) -> Result<(), ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let (actor_roles, _) = fetch_user_access(state.db.as_ref(), &actor.id).await?;
    if is_server_admin(&actor_roles) {
        return Ok(());
    }

    // `VATUSA_STAFF` bundles the entire catalogue (#544), so editing it is editing everything —
    // including whatever is added to the catalogue later. Only a server admin may.
    if role_name == "VATUSA_STAFF" {
        return Err(ApiError::Forbidden);
    }

    let held = access_repo::fetch_effective_permissions(pool, &actor.id).await?;
    let before_set: BTreeSet<&String> = before.iter().collect();
    let after_set: BTreeSet<&String> = after.iter().collect();

    for name in before_set.symmetric_difference(&after_set) {
        let holds = held
            .get(name.as_str())
            .is_some_and(|scope| !scope.is_empty());
        if !holds {
            tracing::warn!(
                actor = actor.id.as_str(),
                group = role_name,
                permission = name.as_str(),
                "refused a group edit touching a permission the actor does not hold"
            );
            return Err(ApiError::Forbidden);
        }
    }
    Ok(())
}

fn group_body(row: access_repo::GroupRow, permissions: Vec<String>) -> GroupBody {
    GroupBody {
        system: access_repo::is_system_role(&row.name),
        name: row.name,
        description: row.description,
        permissions,
        user_count: row.user_count,
        service_account_count: row.service_account_count,
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/groups",
    tag = "access",
    responses((status = 200, body = Vec<GroupBody>), (status = 401))
)]
pub async fn list_groups(
    State(state): State<AppState>,
    _permission: RequirePermission<AccessGroupsRead>,
) -> Result<Json<Vec<GroupBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    // Two queries for the whole listing, not one per group.
    let mut by_group: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (role_name, permission_name) in access_repo::fetch_all_group_permissions(pool).await? {
        by_group.entry(role_name).or_default().push(permission_name);
    }
    let out = access_repo::fetch_groups(pool)
        .await?
        .into_iter()
        .map(|row| {
            let permissions = by_group.get(&row.name).cloned().unwrap_or_default();
            group_body(row, permissions)
        })
        .collect();
    Ok(Json(out))
}

#[utoipa::path(
    post,
    path = "/api/v1/admin/groups",
    tag = "access",
    request_body = CreateGroupRequest,
    responses((status = 200, body = GroupBody), (status = 400), (status = 401), (status = 403), (status = 409))
)]
pub async fn create_group(
    State(state): State<AppState>,
    _permission: RequirePermission<AccessGroupsUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    headers: HeaderMap,
    Json(payload): Json<CreateGroupRequest>,
) -> Result<Json<GroupBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    let name = payload.name.trim().to_ascii_uppercase();
    let reason = payload.reason.trim();
    if reason.is_empty()
        || name.is_empty()
        || !name.chars().all(|c| c.is_ascii_uppercase() || c == '_')
    {
        return Err(ApiError::BadRequest);
    }
    // A new group must not shadow a protected name, even if that name does not exist yet.
    if access_repo::is_system_role(&name) || access_repo::fetch_group(pool, &name).await?.is_some()
    {
        return Err(ApiError::Conflict);
    }

    let description = payload
        .description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    access_repo::create_group(&mut tx, &name, description).await?;
    tx.commit().await.map_err(|_| ApiError::Internal)?;

    let row = access_repo::fetch_group(pool, &name)
        .await?
        .ok_or(ApiError::Internal)?;
    let body = group_body(row, Vec::new());

    audit_group(
        pool,
        user,
        &headers,
        GroupAudit {
            action: "CREATE",
            name: &name,
            reason: Some(reason),
            before: None,
            after: Some(&body),
        },
    )
    .await?;
    Ok(Json(body))
}

#[utoipa::path(
    put,
    path = "/api/v1/admin/groups/{name}",
    tag = "access",
    params(("name" = String, Path, description = "Group name")),
    request_body = UpdateGroupRequest,
    responses((status = 200, body = GroupBody), (status = 400), (status = 401), (status = 403), (status = 404))
)]
pub async fn update_group(
    State(state): State<AppState>,
    _permission: RequirePermission<AccessGroupsUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<UpdateGroupRequest>,
) -> Result<Json<GroupBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    let reason = payload.reason.trim();
    if reason.is_empty() {
        return Err(ApiError::BadRequest);
    }
    // Protected groups are refused before anything else: SERVER_ADMIN must never acquire
    // role_permissions rows (it holds everything through the view), and BOT/SERVICE_APP are a live
    // dependency of every service account.
    if access_repo::is_system_role(&name) {
        return Err(ApiError::Forbidden);
    }

    let catalog: BTreeSet<String> = access_repo::fetch_access_catalog_names(pool)
        .await?
        .into_iter()
        .collect();
    let mut requested: Vec<String> = payload
        .permissions
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    requested.sort();
    requested.dedup();
    if let Some(unknown) = requested.iter().find(|name| !catalog.contains(*name)) {
        tracing::warn!(
            permission = unknown.as_str(),
            "unknown permission in group save"
        );
        return Err(ApiError::BadRequest);
    }

    // Doubles as the existence check — a missing group is a 404 here rather than a separate query.
    let before_row = access_repo::fetch_group(pool, &name)
        .await?
        .ok_or(ApiError::NotFound)?;
    let before_permissions = access_repo::fetch_group_permissions(pool, &name).await?;
    let before_body = group_body(before_row, before_permissions.clone());

    enforce_group_scope(&state, user, &name, &before_permissions, &requested).await?;

    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    access_repo::replace_group_permissions(&mut tx, &name, &requested).await?;
    tx.commit().await.map_err(|_| ApiError::Internal)?;

    let after_row = access_repo::fetch_group(pool, &name)
        .await?
        .ok_or(ApiError::Internal)?;
    let after_permissions = access_repo::fetch_group_permissions(pool, &name).await?;
    let body = group_body(after_row, after_permissions);

    audit_group(
        pool,
        user,
        &headers,
        GroupAudit {
            action: "UPDATE",
            name: &name,
            reason: Some(reason),
            before: Some(&before_body),
            after: Some(&body),
        },
    )
    .await?;

    // Every holder's access changed with this one write — tell connected clients to re-read.
    state.publish(crate::realtime::topic::ACCESS_GRANTED);
    Ok(Json(body))
}

#[utoipa::path(
    delete,
    path = "/api/v1/admin/groups/{name}",
    tag = "access",
    params(("name" = String, Path, description = "Group name")),
    responses((status = 204), (status = 401), (status = 403), (status = 404), (status = 409))
)]
pub async fn delete_group(
    State(state): State<AppState>,
    _permission: RequirePermission<AccessGroupsUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<axum::http::StatusCode, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    if access_repo::is_system_role(&name) {
        return Err(ApiError::Forbidden);
    }
    let row = access_repo::fetch_group(pool, &name)
        .await?
        .ok_or(ApiError::NotFound)?;

    // Refused rather than cascaded. The foreign keys are `on delete cascade`, so deleting a held
    // group would silently take every membership and every bundled permission with it, with nothing
    // recording what was lost. Emptying the group first is itself audited.
    if row.user_count > 0 || row.service_account_count > 0 {
        tracing::warn!(
            group = name.as_str(),
            users = row.user_count,
            service_accounts = row.service_account_count,
            "refused to delete a group that still has holders"
        );
        return Err(ApiError::Conflict);
    }

    let before_permissions = access_repo::fetch_group_permissions(pool, &name).await?;
    let before_body = group_body(row, before_permissions);

    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    access_repo::delete_group(&mut tx, &name).await?;
    tx.commit().await.map_err(|_| ApiError::Internal)?;

    audit_group(
        pool,
        user,
        &headers,
        GroupAudit {
            action: "DELETE",
            name: &name,
            reason: None,
            before: Some(&before_body),
            after: None,
        },
    )
    .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// What a group mutation records. Grouped rather than passed as eight arguments, which also let the
/// create/update and delete paths share one writer instead of two near-identical ones.
struct GroupAudit<'a> {
    action: &'a str,
    name: &'a str,
    reason: Option<&'a str>,
    before: Option<&'a GroupBody>,
    after: Option<&'a GroupBody>,
}

/// Audit a group mutation with before/after.
///
/// Uses `resolve_user_actor_id` rather than `fetch_user_actor_id`: the latter returns `None` for a
/// user with no actor row and silently drops attribution, which on a privileged edit is a hole. And
/// the write is propagated with `?` rather than best-effort — if we cannot record who changed a
/// group, we do not change the group.
async fn audit_group(
    pool: &sqlx::PgPool,
    actor: &CurrentUser,
    headers: &HeaderMap,
    entry: GroupAudit<'_>,
) -> Result<(), ApiError> {
    let actor_id = audit_repo::resolve_user_actor_id(pool, &actor.id, &actor.display_name).await?;
    audit_repo::record_audit(
        pool,
        audit_repo::AuditEntry {
            actor_id,
            action: entry.action.to_string(),
            resource_type: "ACCESS_GROUP".to_string(),
            resource_id: Some(entry.name.to_string()),
            artcc_id: None,
            reason: entry.reason.map(ToOwned::to_owned),
            before_state: entry
                .before
                .and_then(|body| serde_json::to_value(body).ok()),
            after_state: entry.after.and_then(|body| serde_json::to_value(body).ok()),
            ip_address: audit_repo::client_ip(headers),
        },
    )
    .await
}

#[cfg(test)]
mod group_tests {
    use sqlx::PgPool;

    use super::*;
    use crate::scope_test_support::{grant, seed_user, test_state};

    fn actor(id: &str) -> CurrentUser {
        CurrentUser {
            id: id.to_string(),
            cid: 0,
            email: String::new(),
            display_name: "Test Actor".to_string(),
            rating: None,
            primary_role: None,
        }
    }

    async fn make_admin(pool: &PgPool, user_id: &str) {
        sqlx::query(
            "insert into access.user_roles (user_id, role_name) values ($1, 'SERVER_ADMIN')",
        )
        .bind(user_id)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn make_group(pool: &PgPool, name: &str) {
        sqlx::query("insert into access.roles (name, description) values ($1, 'test group')")
            .bind(name)
            .execute(pool)
            .await
            .unwrap();
    }

    // ---- AC4: the no-escalation gate. The heart of #545. ----

    /// Case 1 of the issue's three: **editing a group they hold.** The obvious attack — put a
    /// permission you lack into a group you are a member of, and you have granted it to yourself.
    #[sqlx::test]
    async fn a_non_admin_cannot_add_a_permission_they_do_not_hold(pool: PgPool) {
        let user = seed_user(&pool).await;
        grant(&pool, &user, "access.groups.update", None).await;
        make_group(&pool, "TEST_GROUP").await;
        sqlx::query("insert into access.user_roles (user_id, role_name) values ($1, 'TEST_GROUP')")
            .bind(&user)
            .execute(&pool)
            .await
            .unwrap();
        let state = test_state(pool, std::collections::HashMap::new());

        let result = enforce_group_scope(
            &state,
            &actor(&user),
            "TEST_GROUP",
            &[],
            &["tmu.program.update".to_string()],
        )
        .await;

        assert!(
            matches!(result, Err(ApiError::Forbidden)),
            "adding a permission the actor lacks must be refused, got {result:?}"
        );
    }

    /// Case 2: **editing a group they do not hold.** Still refused — the risk is not only
    /// self-service escalation, it is handing authority to anyone else in that group.
    #[sqlx::test]
    async fn a_non_admin_cannot_arm_a_group_they_are_not_in(pool: PgPool) {
        let user = seed_user(&pool).await;
        grant(&pool, &user, "access.groups.update", None).await;
        make_group(&pool, "OTHER_GROUP").await;
        let state = test_state(pool, std::collections::HashMap::new());

        let result = enforce_group_scope(
            &state,
            &actor(&user),
            "OTHER_GROUP",
            &[],
            &["tmu.program.update".to_string()],
        )
        .await;
        assert!(matches!(result, Err(ApiError::Forbidden)));
    }

    /// Case 3: **create a group, then put something in it.** The create endpoint takes no
    /// permissions, so the gate runs on the first `PUT` — this proves a freshly created group is not
    /// a gap in it.
    #[sqlx::test]
    async fn a_freshly_created_group_is_still_gated(pool: PgPool) {
        let user = seed_user(&pool).await;
        grant(&pool, &user, "access.groups.update", None).await;
        make_group(&pool, "BRAND_NEW").await;
        let state = test_state(pool, std::collections::HashMap::new());

        assert!(matches!(
            enforce_group_scope(
                &state,
                &actor(&user),
                "BRAND_NEW",
                &[],
                &["access.users.update".to_string()]
            )
            .await,
            Err(ApiError::Forbidden)
        ));
    }

    /// What the actor *may* do: put in something they hold.
    #[sqlx::test]
    async fn a_non_admin_may_add_a_permission_they_hold(pool: PgPool) {
        let user = seed_user(&pool).await;
        grant(&pool, &user, "access.groups.update", None).await;
        grant(&pool, &user, "tmu.program.update", None).await;
        make_group(&pool, "TEST_GROUP").await;
        let state = test_state(pool, std::collections::HashMap::new());

        assert!(
            enforce_group_scope(
                &state,
                &actor(&user),
                "TEST_GROUP",
                &[],
                &["tmu.program.update".to_string()]
            )
            .await
            .is_ok()
        );
    }

    /// The symmetric-difference property, copied from `enforce_actor_scope`: **removing** a
    /// permission also requires holding it. Otherwise a facility EC could quietly strip a national
    /// capability out of a group and nobody would have authorised it.
    #[sqlx::test]
    async fn a_non_admin_cannot_remove_a_permission_they_do_not_hold(pool: PgPool) {
        let user = seed_user(&pool).await;
        grant(&pool, &user, "access.groups.update", None).await;
        make_group(&pool, "TEST_GROUP").await;
        let state = test_state(pool, std::collections::HashMap::new());

        let result = enforce_group_scope(
            &state,
            &actor(&user),
            "TEST_GROUP",
            &["tmu.program.update".to_string()],
            &[],
        )
        .await;
        assert!(
            matches!(result, Err(ApiError::Forbidden)),
            "you cannot strip authority you do not hold"
        );
    }

    /// **The reason this gate reads #543's resolver rather than mirroring `enforce_actor_scope`.**
    ///
    /// A permission the actor has been explicitly **denied** still appears in the resolver's map — with
    /// an *empty* scope. So "is it in the map" is not the question; "do they hold it anywhere" is. A
    /// bare `contains_key` passes every other test in this module, which is exactly how the deny-blind
    /// flaw in #559 would creep back in, so this case pins the distinction.
    #[sqlx::test]
    async fn a_denied_permission_is_not_held_however_it_was_granted(pool: PgPool) {
        let user = seed_user(&pool).await;
        grant(&pool, &user, "access.groups.update", None).await;
        // Granted through a group, then denied directly — the only reachable shape, since
        // `user_permissions` is unique on (user, permission, scope) so a direct allow and a direct
        // deny cannot coexist (#544).
        make_group(&pool, "DENY_SRC").await;
        sqlx::query(
            "insert into access.role_permissions (role_name, permission_name) \
             values ('DENY_SRC', 'tmu.program.update')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("insert into access.user_roles (user_id, role_name) values ($1, 'DENY_SRC')")
            .bind(&user)
            .execute(&pool)
            .await
            .unwrap();
        crate::scope_test_support::deny_scoped(&pool, &user, "tmu.program.update", None).await;

        make_group(&pool, "TARGET").await;
        let state = test_state(pool, std::collections::HashMap::new());

        let result = enforce_group_scope(
            &state,
            &actor(&user),
            "TARGET",
            &[],
            &["tmu.program.update".to_string()],
        )
        .await;
        assert!(
            matches!(result, Err(ApiError::Forbidden)),
            "a denied permission must not be grantable through a group, got {result:?}"
        );
    }

    /// `VATUSA_STAFF` bundles the whole catalogue (#544), so editing it is editing everything —
    /// including permissions added after this check was written. Server admins only.
    #[sqlx::test]
    async fn a_non_admin_cannot_edit_the_everything_group(pool: PgPool) {
        let user = seed_user(&pool).await;
        grant(&pool, &user, "access.groups.update", None).await;
        // Even a permission they *do* hold is refused, because the group itself is the problem.
        grant(&pool, &user, "tmu.program.update", None).await;
        let state = test_state(pool, std::collections::HashMap::new());

        assert!(matches!(
            enforce_group_scope(
                &state,
                &actor(&user),
                "VATUSA_STAFF",
                &[],
                &["tmu.program.update".to_string()]
            )
            .await,
            Err(ApiError::Forbidden)
        ));
    }

    /// A server admin bypasses the gate — they already hold everything implicitly.
    #[sqlx::test]
    async fn a_server_admin_may_edit_anything(pool: PgPool) {
        let user = seed_user(&pool).await;
        make_admin(&pool, &user).await;
        let state = test_state(pool, std::collections::HashMap::new());

        assert!(
            enforce_group_scope(
                &state,
                &actor(&user),
                "VATUSA_STAFF",
                &[],
                &["access.users.update".to_string()]
            )
            .await
            .is_ok()
        );
    }

    // ---- AC3: system groups ----

    /// Each of the four is protected, and by name rather than by `is_system` — which defaults to
    /// `true` for every seeded row and so could not distinguish them (#545).
    #[test]
    fn the_four_system_groups_are_protected() {
        for name in ["SERVER_ADMIN", "USER", "BOT", "SERVICE_APP"] {
            assert!(
                access_repo::is_system_role(name),
                "{name} must be protected"
            );
        }
        for name in ["EC", "AEC", "NTMO", "VATUSA_STAFF", "ANYTHING_ELSE"] {
            assert!(!access_repo::is_system_role(name), "{name} is editable");
        }
    }

    // ---- AC1: a created group is immediately assignable ----

    /// The reason the hardcoded const had to go: a group created here must be assignable to users
    /// without a deploy, and `ASSIGNABLE_USER_ROLES` could never see it.
    #[sqlx::test]
    async fn a_new_group_is_assignable_without_a_deploy(pool: PgPool) {
        let before = access_repo::fetch_assignable_role_names(&pool)
            .await
            .unwrap();
        assert!(!before.iter().any(|r| r == "NEW_TEAM"));

        make_group(&pool, "NEW_TEAM").await;

        let after = access_repo::fetch_assignable_role_names(&pool)
            .await
            .unwrap();
        assert!(
            after.iter().any(|r| r == "NEW_TEAM"),
            "a created group must be assignable at once, got {after:?}"
        );
        // And the system groups are never offered.
        for name in access_repo::SYSTEM_ROLES {
            assert!(
                !after.iter().any(|r| r == name),
                "{name} must not be assignable"
            );
        }
    }

    /// The DB-derived list must reproduce the old constant exactly for the groups that existed
    /// before #545 — otherwise switching the user editor over would silently change who can be
    /// assigned what.
    #[sqlx::test]
    async fn the_derived_list_matches_the_old_constant(pool: PgPool) {
        let derived: std::collections::BTreeSet<String> =
            access_repo::fetch_assignable_role_names(&pool)
                .await
                .unwrap()
                .into_iter()
                .collect();
        let legacy: std::collections::BTreeSet<String> = access_repo::ASSIGNABLE_USER_ROLES
            .iter()
            .map(|r| (*r).to_string())
            .collect();
        assert_eq!(derived, legacy);
    }
}
