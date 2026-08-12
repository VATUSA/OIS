//! The access-editor backend: read the catalog, read a user's access, and save
//! roles + permission grants with a required reason (audited). Ported from osmium's
//! admin access handlers; national (unscoped) grants only for now — the `artcc_id`
//! scope dimension is present in the schema and enforced per-domain later.

use std::collections::BTreeSet;

use axum::{
    Json,
    extract::{Extension, Path, State},
    http::HeaderMap,
};

use crate::{
    auth::{
        acl::{
            PermissionPath, fetch_user_access, is_server_admin, normalize_permission_tree,
            permission_tree_from_names, permission_tree_from_paths,
        },
        context::CurrentUser,
        permissions::{AccessCatalogRead, AccessSelfRead, AccessUsersRead, AccessUsersUpdate},
        require_permission::RequirePermission,
    },
    errors::ApiError,
    models::{AccessCatalogBody, SelfAccessBody, UpdateUserAccessRequest, UserAccessBody},
    repos::{access as access_repo, audit as audit_repo},
    state::AppState,
};

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
    }))
}

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
        permissions: permission_tree_from_paths(&permissions),
    }))
}

pub async fn get_user_access(
    State(state): State<AppState>,
    _permission: RequirePermission<AccessUsersRead>,
    Path(cid): Path<i64>,
) -> Result<Json<UserAccessBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let target = access_repo::find_current_user_by_cid(pool, cid)
        .await?
        .ok_or(ApiError::NotFound)?;
    let (roles, permissions) = fetch_user_access(state.db.as_ref(), &target.id).await?;
    Ok(Json(build_user_access_body(&target, &roles, permissions)))
}

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

    // Validate + normalize the edited tree, and reject unknown permission names.
    let requested_names =
        normalize_permission_tree(&payload.permissions).map_err(|_| ApiError::BadRequest)?;
    let catalog: BTreeSet<String> = access_repo::fetch_access_catalog_names(pool)
        .await?
        .into_iter()
        .collect();
    if let Some(unknown) = requested_names.iter().find(|name| !catalog.contains(*name)) {
        tracing::warn!(
            permission = unknown.as_str(),
            "unknown permission in access save"
        );
        return Err(ApiError::BadRequest);
    }
    let requested_permissions =
        access_repo::permission_names_to_permissions(requested_names.clone())?;

    // Reject unknown role names.
    if let Some(role_names) = payload.role_names.as_ref() {
        for role_name in role_names {
            if !access_repo::ASSIGNABLE_USER_ROLES.contains(&role_name.as_str()) {
                return Err(ApiError::BadRequest);
            }
        }
    }

    let target_user_id = access_repo::find_user_id_by_cid(pool, cid)
        .await?
        .ok_or(ApiError::NotFound)?;
    let target_before = access_repo::find_current_user_by_cid(pool, cid)
        .await?
        .ok_or(ApiError::NotFound)?;
    let (before_roles, before_permissions) =
        fetch_user_access(state.db.as_ref(), &target_before.id).await?;

    // Self-scope guard: a non-SERVER_ADMIN actor may only add/remove *direct* grants
    // they themselves effectively hold. Diff against existing direct grants so a
    // target's role-derived or other-granted permissions aren't disturbed.
    let existing_direct_names =
        access_repo::fetch_user_direct_permission_names(pool, &target_user_id).await?;
    let existing_direct = access_repo::permission_names_to_permissions(existing_direct_names)?;
    validate_permission_changes_within_scope(
        &state,
        user,
        &existing_direct,
        &requested_permissions,
    )
    .await?;

    // ...and may only grant/revoke roles they themselves hold.
    if let Some(role_names) = payload.role_names.as_ref() {
        let (actor_roles, _) = fetch_user_access(state.db.as_ref(), &user.id).await?;
        if !is_server_admin(&actor_roles) {
            for role_name in role_names {
                if !actor_roles.contains(role_name) {
                    return Err(ApiError::Forbidden);
                }
            }
        }
    }

    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    access_repo::replace_user_permissions(&mut tx, &target_user_id, &requested_names).await?;
    if let Some(role_names) = payload.role_names.as_ref() {
        for role_name in access_repo::ASSIGNABLE_USER_ROLES {
            let held = role_names.iter().any(|r| r == role_name);
            access_repo::set_user_role_manual(&mut tx, &target_user_id, role_name, held).await?;
        }
    }
    tx.commit().await.map_err(|_| ApiError::Internal)?;

    let updated = access_repo::find_current_user_by_cid(pool, cid)
        .await?
        .ok_or(ApiError::NotFound)?;
    let (roles, permissions) = fetch_user_access(state.db.as_ref(), &updated.id).await?;
    let response = build_user_access_body(&updated, &roles, permissions);

    let actor_id = audit_repo::fetch_user_actor_id(pool, &user.id).await?;
    audit_repo::record_audit(
        pool,
        audit_repo::AuditEntry {
            actor_id,
            action: "UPDATE".to_string(),
            resource_type: "USER_ACCESS".to_string(),
            resource_id: Some(updated.id.clone()),
            artcc_id: None,
            reason: Some(reason.to_string()),
            before_state: Some(serde_json::json!({
                "server_admin": is_server_admin(&before_roles),
                "role_names": before_roles,
                "permissions": permission_tree_from_paths(&before_permissions),
            })),
            after_state: Some(serde_json::json!({
                "server_admin": response.server_admin,
                "role_names": response.role_names,
                "permissions": response.permissions,
            })),
            ip_address: audit_repo::client_ip(&headers),
        },
    )
    .await?;

    Ok(Json(response))
}

async fn validate_permission_changes_within_scope(
    state: &AppState,
    actor: &CurrentUser,
    existing_direct: &[PermissionPath],
    requested: &[PermissionPath],
) -> Result<(), ApiError> {
    let (actor_roles, actor_permissions) = fetch_user_access(state.db.as_ref(), &actor.id).await?;
    if is_server_admin(&actor_roles) {
        return Ok(());
    }

    let existing_set: BTreeSet<&PermissionPath> = existing_direct.iter().collect();
    let requested_set: BTreeSet<&PermissionPath> = requested.iter().collect();
    let actor_set: BTreeSet<&PermissionPath> = actor_permissions.iter().collect();

    for changed in requested_set.symmetric_difference(&existing_set) {
        if !actor_set.contains(changed) {
            return Err(ApiError::Unauthorized);
        }
    }
    Ok(())
}

fn build_user_access_body(
    user: &CurrentUser,
    roles: &[String],
    permissions: Vec<PermissionPath>,
) -> UserAccessBody {
    UserAccessBody {
        id: user.id.clone(),
        cid: user.cid,
        server_admin: is_server_admin(roles),
        role_names: roles.to_vec(),
        permissions: permission_tree_from_paths(&permissions),
    }
}
