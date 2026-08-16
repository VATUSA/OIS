//! Multiple named dashboards per user, optional collections, and share-by-slug. Every owner-scoped
//! query filters `owner_id`, so ownership is enforced here (there is no ownership guard). See
//! migration 0035 and handlers/dashboards.rs.

use serde_json::Value;
use sqlx::PgPool;

use crate::{
    errors::ApiError,
    models::{DashboardBody, DashboardCollection, DashboardSummary, SharedDashboardBody},
};

const COLS: &str = "id, name, collection_id, share_slug, data, updated_at";

pub async fn list_dashboards(
    pool: &PgPool,
    owner: &str,
) -> Result<Vec<DashboardSummary>, ApiError> {
    sqlx::query_as::<_, DashboardSummary>(
        "select id, name, collection_id, share_slug, updated_at from identity.dashboards \
         where owner_id = $1 order by updated_at desc",
    )
    .bind(owner)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn list_collections(
    pool: &PgPool,
    owner: &str,
) -> Result<Vec<DashboardCollection>, ApiError> {
    sqlx::query_as::<_, DashboardCollection>(
        "select id, name from identity.dashboard_collections where owner_id = $1 order by name",
    )
    .bind(owner)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn count_dashboards(pool: &PgPool, owner: &str) -> Result<i64, ApiError> {
    sqlx::query_scalar::<_, i64>("select count(*) from identity.dashboards where owner_id = $1")
        .bind(owner)
        .fetch_one(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

pub async fn get_dashboard(
    pool: &PgPool,
    owner: &str,
    id: &str,
) -> Result<Option<DashboardBody>, ApiError> {
    sqlx::query_as::<_, DashboardBody>(&format!(
        "select {COLS} from identity.dashboards where id = $1 and owner_id = $2"
    ))
    .bind(id)
    .bind(owner)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn create_dashboard(
    pool: &PgPool,
    owner: &str,
    name: &str,
    data: Option<&Value>,
    collection_id: Option<&str>,
) -> Result<DashboardBody, ApiError> {
    sqlx::query_as::<_, DashboardBody>(&format!(
        "insert into identity.dashboards (owner_id, name, data, collection_id) \
         values ($1, $2, coalesce($3, '{{}}'::jsonb), $4) returning {COLS}"
    ))
    .bind(owner)
    .bind(name)
    .bind(data)
    .bind(collection_id)
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Update name/data/collection. `name`/`data` = None leaves them; `collection_id` = None leaves it,
/// `Some("")` clears it (no collection), `Some(id)` moves it.
pub async fn update_dashboard(
    pool: &PgPool,
    owner: &str,
    id: &str,
    name: Option<&str>,
    data: Option<&Value>,
    collection_id: Option<&str>,
) -> Result<Option<DashboardBody>, ApiError> {
    sqlx::query_as::<_, DashboardBody>(&format!(
        "update identity.dashboards set \
            name = coalesce($3, name), \
            data = coalesce($4, data), \
            collection_id = case when $5::text is null then collection_id \
                                 when $5 = '' then null else $5 end, \
            updated_at = now() \
         where id = $1 and owner_id = $2 returning {COLS}"
    ))
    .bind(id)
    .bind(owner)
    .bind(name)
    .bind(data)
    .bind(collection_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn delete_dashboard(pool: &PgPool, owner: &str, id: &str) -> Result<bool, ApiError> {
    let r = sqlx::query("delete from identity.dashboards where id = $1 and owner_id = $2")
        .bind(id)
        .bind(owner)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(r.rows_affected() > 0)
}

/// Set the share slug if unset (idempotent), returning the effective slug — or `None` if the board
/// isn't the caller's.
pub async fn set_share(
    pool: &PgPool,
    owner: &str,
    id: &str,
    new_slug: &str,
) -> Result<Option<String>, ApiError> {
    sqlx::query_scalar::<_, String>(
        "update identity.dashboards set share_slug = coalesce(share_slug, $3), updated_at = now() \
         where id = $1 and owner_id = $2 returning share_slug",
    )
    .bind(id)
    .bind(owner)
    .bind(new_slug)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn clear_share(pool: &PgPool, owner: &str, id: &str) -> Result<bool, ApiError> {
    let r = sqlx::query(
        "update identity.dashboards set share_slug = null, updated_at = now() \
         where id = $1 and owner_id = $2",
    )
    .bind(id)
    .bind(owner)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(r.rows_affected() > 0)
}

pub async fn get_shared(
    pool: &PgPool,
    slug: &str,
) -> Result<Option<SharedDashboardBody>, ApiError> {
    sqlx::query_as::<_, SharedDashboardBody>(
        "select d.name, u.display_name as owner, d.data from identity.dashboards d \
         join identity.users u on u.id = d.owner_id where d.share_slug = $1",
    )
    .bind(slug)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Clone a shared board into `new_owner`'s library, returning the new id (or `None` for a bad slug).
pub async fn copy_shared(
    pool: &PgPool,
    slug: &str,
    new_owner: &str,
) -> Result<Option<String>, ApiError> {
    sqlx::query_scalar::<_, String>(
        "insert into identity.dashboards (owner_id, name, data, collection_id) \
         select $2, name || ' (copy)', data, null from identity.dashboards where share_slug = $1 \
         returning id",
    )
    .bind(slug)
    .bind(new_owner)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn create_collection(
    pool: &PgPool,
    owner: &str,
    name: &str,
) -> Result<DashboardCollection, ApiError> {
    sqlx::query_as::<_, DashboardCollection>(
        "insert into identity.dashboard_collections (owner_id, name) values ($1, $2) \
         returning id, name",
    )
    .bind(owner)
    .bind(name)
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn rename_collection(
    pool: &PgPool,
    owner: &str,
    id: &str,
    name: &str,
) -> Result<bool, ApiError> {
    let r = sqlx::query(
        "update identity.dashboard_collections set name = $3 where id = $1 and owner_id = $2",
    )
    .bind(id)
    .bind(owner)
    .bind(name)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(r.rows_affected() > 0)
}

pub async fn delete_collection(pool: &PgPool, owner: &str, id: &str) -> Result<bool, ApiError> {
    let r =
        sqlx::query("delete from identity.dashboard_collections where id = $1 and owner_id = $2")
            .bind(id)
            .bind(owner)
            .execute(pool)
            .await
            .map_err(|_| ApiError::Internal)?;
    Ok(r.rows_affected() > 0)
}
