//! Per-facility reference documents (SOPs/LOAs/etc.). Reads are gated by `facilities.docs.read`;
//! writes are facility-scoped by `facilities.docs.update` (same scope infra as
//! `events.config.update`). Consumed by the Discord ACE-claim DM (a later sub-issue of #143).

use axum::{
    Json,
    extract::{Extension, Path, State},
    http::StatusCode,
};

use crate::{
    auth::{
        context::{CurrentApiKey, CurrentUser},
        permissions::{FacilitiesDocsRead, FacilitiesDocsUpdate},
        principal::Principal,
        require_permission::RequirePermission,
    },
    errors::ApiError,
    models::{FacilityDocumentBody, UpsertFacilityDocumentRequest},
    repos::facility_documents as doc_repo,
    state::AppState,
};

const UPDATE_PERMISSION: &str = "facilities.docs.update";

fn validate(req: &UpsertFacilityDocumentRequest) -> Result<(), ApiError> {
    if req.title.trim().is_empty() || req.title.len() > 200 {
        return Err(ApiError::BadRequest);
    }
    if req.url.trim().is_empty() || req.url.len() > 2000 {
        return Err(ApiError::BadRequest);
    }
    Ok(())
}

#[utoipa::path(
    get, path = "/api/v1/facilities/{facility_id}/documents", tag = "facilities",
    params(("facility_id" = String, Path)),
    responses((status = 200, body = Vec<FacilityDocumentBody>), (status = 401))
)]
pub async fn list_facility_documents(
    State(state): State<AppState>,
    _permission: RequirePermission<FacilitiesDocsRead>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path(facility_id): Path<String>,
) -> Result<Json<Vec<FacilityDocumentBody>>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    let scope = principal
        .permission_scope(&state, UPDATE_PERMISSION)
        .await?;
    let editable = scope.allows(Some(&facility_id));
    let mut rows = doc_repo::list_by_facility(pool, &facility_id).await?;
    for r in &mut rows {
        r.editable = editable;
    }
    Ok(Json(rows))
}

#[utoipa::path(
    post, path = "/api/v1/facilities/{facility_id}/documents", tag = "facilities",
    params(("facility_id" = String, Path)), request_body = UpsertFacilityDocumentRequest,
    responses((status = 200, body = FacilityDocumentBody), (status = 400), (status = 401), (status = 403))
)]
pub async fn create_facility_document(
    State(state): State<AppState>,
    _permission: RequirePermission<FacilitiesDocsUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path(facility_id): Path<String>,
    Json(req): Json<UpsertFacilityDocumentRequest>,
) -> Result<Json<FacilityDocumentBody>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    validate(&req)?;

    let scope = principal
        .permission_scope(&state, UPDATE_PERMISSION)
        .await?;
    if !scope.allows(Some(&facility_id)) {
        return Err(ApiError::Forbidden);
    }

    let mut row = doc_repo::create(pool, &facility_id, &req).await?;
    row.editable = true;
    Ok(Json(row))
}

#[utoipa::path(
    put, path = "/api/v1/facilities/{facility_id}/documents/{id}", tag = "facilities",
    params(("facility_id" = String, Path), ("id" = String, Path)),
    request_body = UpsertFacilityDocumentRequest,
    responses((status = 200, body = FacilityDocumentBody), (status = 400), (status = 401), (status = 403), (status = 404))
)]
pub async fn update_facility_document(
    State(state): State<AppState>,
    _permission: RequirePermission<FacilitiesDocsUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path((facility_id, id)): Path<(String, String)>,
    Json(req): Json<UpsertFacilityDocumentRequest>,
) -> Result<Json<FacilityDocumentBody>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    validate(&req)?;

    let scope = principal
        .permission_scope(&state, UPDATE_PERMISSION)
        .await?;
    if !scope.allows(Some(&facility_id)) {
        return Err(ApiError::Forbidden);
    }

    let mut row = doc_repo::update(pool, &id, &facility_id, &req)
        .await?
        .ok_or(ApiError::NotFound)?;
    row.editable = true;
    Ok(Json(row))
}

#[utoipa::path(
    delete, path = "/api/v1/facilities/{facility_id}/documents/{id}", tag = "facilities",
    params(("facility_id" = String, Path), ("id" = String, Path)),
    responses((status = 204), (status = 401), (status = 403), (status = 404))
)]
pub async fn delete_facility_document(
    State(state): State<AppState>,
    _permission: RequirePermission<FacilitiesDocsUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path((facility_id, id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    let scope = principal
        .permission_scope(&state, UPDATE_PERMISSION)
        .await?;
    if !scope.allows(Some(&facility_id)) {
        return Err(ApiError::Forbidden);
    }

    if doc_repo::delete(pool, &id, &facility_id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use sqlx::PgPool;

    use super::*;
    use crate::repos::access::PermissionScope;

    fn upsert(title: &str) -> UpsertFacilityDocumentRequest {
        UpsertFacilityDocumentRequest {
            title: title.to_string(),
            url: "https://example.com/doc.pdf".to_string(),
        }
    }

    #[sqlx::test]
    async fn crud_round_trip(pool: PgPool) {
        let created = doc_repo::create(&pool, "ZDC", &upsert("ZDC SOP"))
            .await
            .unwrap();
        assert_eq!(created.facility_id, "ZDC");
        assert_eq!(created.title, "ZDC SOP");

        let listed = doc_repo::list_by_facility(&pool, "ZDC").await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, created.id);

        let updated = doc_repo::update(&pool, &created.id, "ZDC", &upsert("ZDC SOP v2"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.title, "ZDC SOP v2");

        // Wrong facility_id in the WHERE clause — no row matches, update is a no-op.
        let mismatched = doc_repo::update(&pool, &created.id, "ZAU", &upsert("nope"))
            .await
            .unwrap();
        assert!(mismatched.is_none());

        // Wrong facility_id on delete, too — a ZDC document can't be deleted via a ZAU-scoped call.
        assert!(!doc_repo::delete(&pool, &created.id, "ZAU").await.unwrap());
        assert!(doc_repo::delete(&pool, &created.id, "ZDC").await.unwrap());
        assert!(
            doc_repo::list_by_facility(&pool, "ZDC")
                .await
                .unwrap()
                .is_empty()
        );
    }

    /// A facility-scoped principal may edit only their own facility's documents; a national scope
    /// covers every facility. Mirrors the equivalent scoping test in `handlers::airport_configs`.
    #[sqlx::test]
    async fn facility_scope_gates_editability(_pool: PgPool) {
        let zdc_scope = PermissionScope::Facilities(HashSet::from(["ZDC".to_string()]));
        assert!(zdc_scope.allows(Some("ZDC")));
        assert!(!zdc_scope.allows(Some("ZAU")));

        assert!(PermissionScope::National.allows(Some("ZDC")));
        assert!(PermissionScope::National.allows(Some("ZAU")));

        let empty_scope = PermissionScope::Facilities(HashSet::new());
        assert!(!empty_scope.allows(Some("ZDC")));
    }
}
