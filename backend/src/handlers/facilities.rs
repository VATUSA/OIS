//! Facilities (ARTCC) directory — public reference data.

use axum::{
    Json,
    extract::{Path, State},
};

use crate::{errors::ApiError, models::FacilityBody, repos::org as org_repo, state::AppState};

#[utoipa::path(
    get,
    path = "/api/v1/facilities",
    tag = "facilities",
    responses((status = 200, body = Vec<FacilityBody>))
)]
pub async fn list_facilities(
    State(state): State<AppState>,
) -> Result<Json<Vec<FacilityBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(org_repo::list_facilities(pool).await?))
}

#[utoipa::path(
    get,
    path = "/api/v1/facilities/{id}",
    tag = "facilities",
    params(("id" = String, Path, description = "ARTCC id (e.g. ZDC)")),
    responses((status = 200, body = FacilityBody), (status = 404))
)]
pub async fn get_facility(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<FacilityBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let facility = org_repo::find_facility(pool, &id.to_ascii_uppercase())
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(facility))
}
