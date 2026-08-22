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

/// The audit actor id for a non-user principal (`api_key`/`service_account`), keyed on the given
/// id column. Creates the actor row if it doesn't exist yet, then returns its id. `id_column` is a
/// trusted internal constant (never user input); `actor_type` and `principal_id` are bound params.
async fn resolve_principal_actor_id(
    pool: &PgPool,
    actor_type: &str,
    id_column: &str,
    principal_id: &str,
    display_name: &str,
) -> Result<Option<String>, ApiError> {
    let select =
        format!("select id from access.actors where actor_type = $1 and {id_column} = $2 limit 1");
    if let Some(id) = sqlx::query_scalar::<_, String>(&select)
        .bind(actor_type)
        .bind(principal_id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)?
    {
        return Ok(Some(id));
    }
    let insert = format!(
        "insert into access.actors (actor_type, {id_column}, display_name) \
         select $1, $2, $3 \
         where not exists ( \
             select 1 from access.actors where actor_type = $1 and {id_column} = $2 \
         )"
    );
    sqlx::query(&insert)
        .bind(actor_type)
        .bind(principal_id)
        .bind(display_name)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    sqlx::query_scalar::<_, String>(&select)
        .bind(actor_type)
        .bind(principal_id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// An API key's audit actor id if one exists (read-only; does not create). Returns None if the key
/// has never acted — used to render a per-key activity dossier.
pub async fn fetch_api_key_actor_id(
    pool: &PgPool,
    api_key_id: &str,
) -> Result<Option<String>, ApiError> {
    sqlx::query_scalar::<_, String>(
        "select id from access.actors where actor_type = 'api_key' and api_key_id = $1 limit 1",
    )
    .bind(api_key_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// An API key's audit actor id, creating the actor row if needed. So every mutation a key makes is
/// attributed to the key (identified by its `display_name` = name + prefix), not left unlogged.
pub async fn resolve_api_key_actor_id(
    pool: &PgPool,
    api_key_id: &str,
    display_name: &str,
) -> Result<Option<String>, ApiError> {
    resolve_principal_actor_id(pool, "api_key", "api_key_id", api_key_id, display_name).await
}

/// A service account's audit actor id, creating the actor row if needed. Closes the gap where
/// bearer (bot) mutations went unattributed.
pub async fn resolve_service_account_actor_id(
    pool: &PgPool,
    service_account_id: &str,
    display_name: &str,
) -> Result<Option<String>, ApiError> {
    resolve_principal_actor_id(
        pool,
        "service_account",
        "service_account_id",
        service_account_id,
        display_name,
    )
    .await
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
    pub resource_id: Option<String>,
    pub action: Option<String>,
    /// Restrict to one audit actor (a user, api key, or service-account actor row) — the basis of a
    /// per-key or per-user activity dossier.
    pub actor_id: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

pub async fn count_audit_logs(pool: &PgPool, filters: &AuditLogFilters) -> Result<i64, ApiError> {
    sqlx::query_scalar::<_, i64>(
        "select count(*) from access.audit_logs \
         where ($1::text is null or resource_type = $1) \
           and ($2::text is null or action = $2) \
           and ($3::text is null or resource_id = $3) \
           and ($4::text is null or actor_id = $4)",
    )
    .bind(filters.resource_type.as_deref())
    .bind(filters.action.as_deref())
    .bind(filters.resource_id.as_deref())
    .bind(filters.actor_id.as_deref())
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
    actor_type: Option<String>,
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
    // `actor_display_name` falls back to the actor row's own label so api-key and service-account
    // actors (which have no linked user) still read as a name, not a blank.
    let rows = sqlx::query_as::<_, AuditLogRow>(
        "select l.id, l.action, l.resource_type, l.resource_id, l.artcc_id, l.reason, \
                a.actor_type as actor_type, \
                u.cid as actor_cid, coalesce(u.display_name, a.display_name) as actor_display_name, \
                l.before_state::text as before_state, l.after_state::text as after_state, \
                l.created_at \
         from access.audit_logs l \
         left join access.actors a on a.id = l.actor_id \
         left join identity.users u on u.id = a.user_id \
         where ($1::text is null or l.resource_type = $1) \
           and ($2::text is null or l.action = $2) \
           and ($3::text is null or l.resource_id = $3) \
           and ($4::text is null or l.actor_id = $4) \
         order by l.created_at desc \
         limit $5 offset $6",
    )
    .bind(filters.resource_type.as_deref())
    .bind(filters.action.as_deref())
    .bind(filters.resource_id.as_deref())
    .bind(filters.actor_id.as_deref())
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
            actor_type: row.actor_type,
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
