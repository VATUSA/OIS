//! Facility map color-rule configuration. The map itself is public, so reads are open (no auth) and
//! carry an `editable` flag resolved from the caller's scope. Writes are facility-scoped by the
//! facility's own ARTCC id (`flow.facility_map.update`), reusing the same scope infra as airport
//! configs — but here `facility_id` *is* the ARTCC, so no owning-ARTCC lookup is needed.

use axum::{
    Json,
    extract::{Extension, Path, State},
};

use crate::{
    auth::{
        context::{CurrentApiKey, CurrentUser},
        permissions::FlowFacilityMapUpdate,
        principal::Principal,
        require_permission::RequirePermission,
    },
    errors::ApiError,
    models::{FacilityMapConfigBody, UpsertFacilityMapConfigRequest},
    repos::facility_map as config_repo,
    state::AppState,
};

const CONFIG_PERMISSION: &str = "flow.facility_map.update";

/// Normalize a facility id to an uppercase ARTCC code (2–4 alphanumerics).
fn normalize_facility(id: &str) -> Option<String> {
    let id = id.trim().to_ascii_uppercase();
    if (2..=4).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric()) {
        Some(id)
    } else {
        None
    }
}

/// Reject obviously-bad rule sets (defends the store; the client engine owns the semantics).
fn validate(req: &UpsertFacilityMapConfigRequest) -> Result<(), ApiError> {
    if req.rules.len() > 100 {
        return Err(ApiError::BadRequest);
    }
    if req.default_color.len() > 16 {
        return Err(ApiError::BadRequest);
    }
    for r in &req.rules {
        if r.id.len() > 64 || r.label.len() > 80 || r.color.len() > 16 || r.conditions.len() > 20 {
            return Err(ApiError::BadRequest);
        }
        for c in &r.conditions {
            if c.field.len() > 16 || c.op.len() > 16 || c.values.len() > 200 {
                return Err(ApiError::BadRequest);
            }
        }
    }
    Ok(())
}

/// Does the caller hold `flow.facility_map.update` nationally or for this facility's ARTCC?
/// Works for a signed-in user or an API key (whose scope is capped by its owner).
async fn can_edit(
    state: &AppState,
    principal: Option<&Principal>,
    facility_id: &str,
) -> Result<bool, ApiError> {
    let Some(principal) = principal else {
        return Ok(false);
    };
    let scope = principal.permission_scope(state, CONFIG_PERMISSION).await?;
    Ok(scope.allows(Some(facility_id)))
}

#[utoipa::path(
    get, path = "/api/v1/facility-map/{id}/config", tag = "flow",
    params(("id" = String, Path)),
    responses((status = 200, body = FacilityMapConfigBody), (status = 400))
)]
pub async fn get_config(
    State(state): State<AppState>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path(id): Path<String>,
) -> Result<Json<FacilityMapConfigBody>, ApiError> {
    let facility_id = normalize_facility(&id).ok_or(ApiError::BadRequest)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    let (rules, default_color) = config_repo::get(pool, &facility_id)
        .await?
        .unwrap_or_default();
    let principal = Principal::optional(current_user.as_ref(), current_api_key.as_ref());
    let editable = can_edit(&state, principal.as_ref(), &facility_id).await?;
    Ok(Json(FacilityMapConfigBody {
        facility_id,
        rules,
        default_color,
        editable,
    }))
}

#[utoipa::path(
    put, path = "/api/v1/facility-map/{id}/config", tag = "flow",
    params(("id" = String, Path)), request_body = UpsertFacilityMapConfigRequest,
    responses((status = 200, body = FacilityMapConfigBody), (status = 400), (status = 401), (status = 403))
)]
pub async fn put_config(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFacilityMapUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path(id): Path<String>,
    Json(req): Json<UpsertFacilityMapConfigRequest>,
) -> Result<Json<FacilityMapConfigBody>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let facility_id = normalize_facility(&id).ok_or(ApiError::BadRequest)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    validate(&req)?;

    let scope = principal
        .permission_scope(&state, CONFIG_PERMISSION)
        .await?;
    if !scope.allows(Some(&facility_id)) {
        return Err(ApiError::Forbidden);
    }

    config_repo::upsert(pool, &facility_id, &req, principal.user_id()).await?;
    Ok(Json(FacilityMapConfigBody {
        facility_id,
        rules: req.rules,
        default_color: req.default_color,
        editable: true,
    }))
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::*;
    use crate::scope_test_support::{self, grant, principal_for, test_state};

    // --- ARTCC-scope authorization boundary (#198) ---
    //
    // This file had no tests at all — `can_edit` (used both for the read-side `editable` flag and,
    // duplicated inline, for `put_config`'s write gate) had zero coverage.

    #[sqlx::test]
    async fn national_scope_can_edit_any_facility(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "flow.facility_map.update", None).await;
        let principal = principal_for(&user);
        let state = test_state(pool, std::collections::HashMap::new());
        assert!(can_edit(&state, Some(&principal), "ZDC").await.unwrap());
    }

    #[sqlx::test]
    async fn matching_facility_scope_can_edit_its_own_facility(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "flow.facility_map.update", Some("ZDC")).await;
        let principal = principal_for(&user);
        let state = test_state(pool, std::collections::HashMap::new());
        assert!(can_edit(&state, Some(&principal), "ZDC").await.unwrap());
    }

    #[sqlx::test]
    async fn wrong_facility_scope_is_rejected(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "flow.facility_map.update", Some("ZAU")).await;
        let principal = principal_for(&user);
        let state = test_state(pool, std::collections::HashMap::new());
        assert!(!can_edit(&state, Some(&principal), "ZDC").await.unwrap());
    }

    #[sqlx::test]
    async fn no_grant_at_all_is_rejected(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        let principal = principal_for(&user);
        let state = test_state(pool, std::collections::HashMap::new());
        assert!(!can_edit(&state, Some(&principal), "ZDC").await.unwrap());
    }

    #[sqlx::test]
    async fn no_principal_at_all_is_rejected(pool: PgPool) {
        let state = test_state(pool, std::collections::HashMap::new());
        assert!(!can_edit(&state, None, "ZDC").await.unwrap());
    }
}
