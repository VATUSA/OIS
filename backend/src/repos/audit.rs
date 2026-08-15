//! Audit-log writes. `access.audit_logs` carries a `reason` column directly, so the
//! access editor's required reason ("Recorded as a dossier entry on this controller's
//! log") is stored on the same row as the before/after snapshot.

use chrono::{DateTime, Utc};
use http::HeaderMap;
use serde_json::Value;
use sqlx::PgPool;

use crate::{errors::ApiError, models::AuditLogEntry};

pub struct AuditEntry {
    pub actor_id: Option<String>,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<String>,
    pub artcc_id: Option<String>,
    pub reason: Option<String>,
    pub before_state: Option<Value>,
    pub after_state: Option<Value>,
    pub ip_address: Option<String>,
}

/// The audit actor id for a user (created at login by `ensure_user_actor`).
pub async fn fetch_user_actor_id(pool: &PgPool, user_id: &str) -> Result<Option<String>, ApiError> {
    sqlx::query_scalar::<_, String>(
        "select id from access.actors where actor_type = 'user' and user_id = $1 limit 1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// The user's audit actor id, creating the actor row if it doesn't exist yet. Used by the
/// audit middleware so an action is still attributed even if the user predates actor seeding.
pub async fn resolve_user_actor_id(
    pool: &PgPool,
    user_id: &str,
    display_name: &str,
) -> Result<Option<String>, ApiError> {
    if let Some(id) = fetch_user_actor_id(pool, user_id).await? {
        return Ok(Some(id));
    }
    sqlx::query(
        "insert into access.actors (actor_type, user_id, display_name) \
         select 'user', $1, $2 \
         where not exists ( \
             select 1 from access.actors where actor_type = 'user' and user_id = $1 \
         )",
    )
    .bind(user_id)
    .bind(display_name)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    fetch_user_actor_id(pool, user_id).await
}

pub async fn record_audit(pool: &PgPool, entry: AuditEntry) -> Result<(), ApiError> {
    // before/after are serialized to text and cast to jsonb so we don't need sqlx's
    // `json` feature. ip is cast to inet (null-safe).
    let before = entry
        .before_state
        .as_ref()
        .map(|value| serde_json::to_string(value).unwrap_or_else(|_| "null".to_string()));
    let after = entry
        .after_state
        .as_ref()
        .map(|value| serde_json::to_string(value).unwrap_or_else(|_| "null".to_string()));

    sqlx::query(
        r#"
        insert into access.audit_logs
            (actor_id, action, resource_type, resource_id, artcc_id, reason,
             before_state, after_state, ip_address)
        values ($1, $2, $3, $4, $5, $6, $7::jsonb, $8::jsonb, $9::inet)
        "#,
    )
    .bind(entry.actor_id)
    .bind(entry.action)
    .bind(entry.resource_type)
    .bind(entry.resource_id)
    .bind(entry.artcc_id)
    .bind(entry.reason)
    .bind(before)
    .bind(after)
    .bind(entry.ip_address)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;

    Ok(())
}

pub struct AuditLogFilters {
    pub resource_type: Option<String>,
    pub action: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

pub async fn count_audit_logs(pool: &PgPool, filters: &AuditLogFilters) -> Result<i64, ApiError> {
    sqlx::query_scalar::<_, i64>(
        "select count(*) from access.audit_logs \
         where ($1::text is null or resource_type = $1) \
           and ($2::text is null or action = $2)",
    )
    .bind(filters.resource_type.as_deref())
    .bind(filters.action.as_deref())
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

#[derive(sqlx::FromRow)]
struct AuditLogRow {
    id: String,
    action: String,
    resource_type: String,
    resource_id: Option<String>,
    artcc_id: Option<String>,
    reason: Option<String>,
    actor_cid: Option<i64>,
    actor_display_name: Option<String>,
    before_state: Option<String>,
    after_state: Option<String>,
    created_at: DateTime<Utc>,
}

pub async fn fetch_audit_logs(
    pool: &PgPool,
    filters: &AuditLogFilters,
) -> Result<Vec<AuditLogEntry>, ApiError> {
    let rows = sqlx::query_as::<_, AuditLogRow>(
        "select l.id, l.action, l.resource_type, l.resource_id, l.artcc_id, l.reason, \
                u.cid as actor_cid, u.display_name as actor_display_name, \
                l.before_state::text as before_state, l.after_state::text as after_state, \
                l.created_at \
         from access.audit_logs l \
         left join access.actors a on a.id = l.actor_id \
         left join identity.users u on u.id = a.user_id \
         where ($1::text is null or l.resource_type = $1) \
           and ($2::text is null or l.action = $2) \
         order by l.created_at desc \
         limit $3 offset $4",
    )
    .bind(filters.resource_type.as_deref())
    .bind(filters.action.as_deref())
    .bind(filters.limit)
    .bind(filters.offset)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;

    Ok(rows
        .into_iter()
        .map(|row| AuditLogEntry {
            id: row.id,
            action: row.action,
            resource_type: row.resource_type,
            resource_id: row.resource_id,
            artcc_id: row.artcc_id,
            reason: row.reason,
            actor_cid: row.actor_cid,
            actor_display_name: row.actor_display_name,
            before_state: row.before_state.and_then(|s| serde_json::from_str(&s).ok()),
            after_state: row.after_state.and_then(|s| serde_json::from_str(&s).ok()),
            created_at: row.created_at,
        })
        .collect())
}

/// Best-effort client IP from proxy headers (first `X-Forwarded-For` hop, else
/// `X-Real-IP`). None in local dev without a proxy.
pub fn client_ip(headers: &HeaderMap) -> Option<String> {
    if let Some(forwarded) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok())
        && let Some(first) = forwarded.split(',').next()
    {
        let trimmed = first.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}
