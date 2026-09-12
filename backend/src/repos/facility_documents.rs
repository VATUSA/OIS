//! Per-facility reference documents (see migration 0064). Writes are facility-scoped in the
//! handler; the repo is unscoped.

use sqlx::PgPool;

use crate::{
    errors::ApiError,
    models::{FacilityDocumentBody, UpsertFacilityDocumentRequest},
};

const DOC_SELECT: &str =
    "select id, facility_id, title, url, updated_at from org.facility_documents";

pub async fn list_by_facility(
    pool: &PgPool,
    facility_id: &str,
) -> Result<Vec<FacilityDocumentBody>, ApiError> {
    sqlx::query_as::<_, FacilityDocumentBody>(&format!(
        "{DOC_SELECT} where facility_id = $1 order by created_at"
    ))
    .bind(facility_id)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn get(pool: &PgPool, id: &str) -> Result<Option<FacilityDocumentBody>, ApiError> {
    sqlx::query_as::<_, FacilityDocumentBody>(&format!("{DOC_SELECT} where id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

pub async fn create(
    pool: &PgPool,
    facility_id: &str,
    req: &UpsertFacilityDocumentRequest,
) -> Result<FacilityDocumentBody, ApiError> {
    let id: String = sqlx::query_scalar(
        "insert into org.facility_documents (facility_id, title, url) \
         values ($1, $2, $3) returning id",
    )
    .bind(facility_id)
    .bind(&req.title)
    .bind(&req.url)
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    get(pool, &id).await?.ok_or(ApiError::Internal)
}

pub async fn update(
    pool: &PgPool,
    id: &str,
    facility_id: &str,
    req: &UpsertFacilityDocumentRequest,
) -> Result<Option<FacilityDocumentBody>, ApiError> {
    let r = sqlx::query(
        "update org.facility_documents set title = $3, url = $4 \
         where id = $1 and facility_id = $2",
    )
    .bind(id)
    .bind(facility_id)
    .bind(&req.title)
    .bind(&req.url)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    if r.rows_affected() == 0 {
        return Ok(None);
    }
    get(pool, id).await
}

pub async fn delete(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    let r = sqlx::query("delete from org.facility_documents where id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(r.rows_affected() > 0)
}
