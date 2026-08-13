//! TMU persistence — Traffic Management Initiatives (TMIs).

use sqlx::PgPool;

use crate::{
    errors::ApiError,
    models::{CreateTmiRequest, TmiBody, UpdateTmiRequest},
};

const SELECT: &str = "select t.id, t.requesting, t.providing, t.restriction, \
    t.start_time, t.stop_time, t.status, t.published_at, t.created_at, \
    u.display_name as author \
    from tmu.tmis t left join identity.users u on u.id = t.created_by";

pub async fn list_tmis(pool: &PgPool, status: Option<&str>) -> Result<Vec<TmiBody>, ApiError> {
    sqlx::query_as::<_, TmiBody>(&format!(
        "{SELECT} where ($1::text is null or t.status = $1) order by t.created_at desc"
    ))
    .bind(status)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn get_tmi(pool: &PgPool, id: &str) -> Result<Option<TmiBody>, ApiError> {
    sqlx::query_as::<_, TmiBody>(&format!("{SELECT} where t.id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

pub async fn create_tmi(
    pool: &PgPool,
    req: &CreateTmiRequest,
    created_by: &str,
) -> Result<String, ApiError> {
    sqlx::query_scalar::<_, String>(
        "insert into tmu.tmis \
         (requesting, providing, restriction, start_time, stop_time, created_by) \
         values ($1, $2, $3, coalesce($4, now()), $5, $6) returning id",
    )
    .bind(&req.requesting)
    .bind(&req.providing)
    .bind(&req.restriction)
    .bind(req.start_time)
    .bind(req.stop_time)
    .bind(created_by)
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Updates the given fields (COALESCE — omitted fields are left unchanged). Returns
/// false if the TMI doesn't exist.
pub async fn update_tmi(pool: &PgPool, id: &str, req: &UpdateTmiRequest) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "update tmu.tmis set \
            requesting = coalesce($2, requesting), \
            providing = coalesce($3, providing), \
            restriction = coalesce($4, restriction), \
            start_time = coalesce($5, start_time), \
            stop_time = coalesce($6, stop_time) \
         where id = $1",
    )
    .bind(id)
    .bind(&req.requesting)
    .bind(&req.providing)
    .bind(&req.restriction)
    .bind(req.start_time)
    .bind(req.stop_time)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

/// Publishes a draft. Returns false if the TMI isn't currently a draft.
pub async fn publish_tmi(pool: &PgPool, id: &str, published_by: &str) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "update tmu.tmis set status = 'published', published_by = $2, published_at = now() \
         where id = $1 and status = 'draft'",
    )
    .bind(id)
    .bind(published_by)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

/// Cancels a draft or published TMI. Returns false if it's already terminal.
pub async fn cancel_tmi(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    let result = sqlx::query(
        "update tmu.tmis set status = 'cancelled' \
         where id = $1 and status in ('draft', 'published')",
    )
    .bind(id)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}

pub async fn delete_tmi(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    let result = sqlx::query("delete from tmu.tmis where id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(result.rows_affected() > 0)
}
