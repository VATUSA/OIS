//! Per-user, per-namespace preferences (opaque client-owned jsonb). See migration 0034.

use serde_json::Value;
use sqlx::PgPool;

use crate::errors::ApiError;

/// Fetch a user's preferences for a namespace, or `None` if unset.
pub async fn get_pref(
    pool: &PgPool,
    user_id: &str,
    namespace: &str,
) -> Result<Option<Value>, ApiError> {
    let row = sqlx::query_scalar::<_, Value>(
        "select value from identity.user_preferences where user_id = $1 and namespace = $2",
    )
    .bind(user_id)
    .bind(namespace)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(row)
}

/// Upsert a user's preferences for a namespace.
pub async fn put_pref(
    pool: &PgPool,
    user_id: &str,
    namespace: &str,
    value: &Value,
) -> Result<(), ApiError> {
    sqlx::query(
        "insert into identity.user_preferences (user_id, namespace, value) \
         values ($1, $2, $3) \
         on conflict (user_id, namespace) do update \
         set value = excluded.value, updated_at = now()",
    )
    .bind(user_id)
    .bind(namespace)
    .bind(value)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(())
}
