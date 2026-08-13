//! The access-editor backend: read the catalog, read a user's access, and save
//! roles + permission grants with a required reason (audited). Ported from osmium's
//! admin access handlers and extended with per-ARTCC scope: grants can be national
//! (`artcc_id = null`) or scoped to a facility.

use std::collections::{BTreeMap, BTreeSet};

use axum::{
    Json,
    extract::{Extension, Path, State},
    http::HeaderMap,
};

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
        AccessCatalogBody, ScopeAccess, SelfAccessBody, UpdateUserAccessRequest, UserAccessBody,
    },
    repos::{access as access_repo, audit as audit_repo, org as org_repo},
    state::AppState,
};

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
        if let Some(artcc_id) = &artcc {
            if !facility_ids.contains(artcc_id) {
                return Err(ApiError::BadRequest);
            }
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
    normalize_permission_tree(tree).map_err(|_| ApiError::BadRequest)
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
