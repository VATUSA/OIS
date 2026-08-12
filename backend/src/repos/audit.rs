//! Audit-log writes. `access.audit_logs` carries a `reason` column directly, so the
//! access editor's required reason ("Recorded as a dossier entry on this controller's
//! log") is stored on the same row as the before/after snapshot.

use http::HeaderMap;
use serde_json::Value;
use sqlx::PgPool;

use crate::errors::ApiError;

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

/// Best-effort client IP from proxy headers (first `X-Forwarded-For` hop, else
/// `X-Real-IP`). None in local dev without a proxy.
pub fn client_ip(headers: &HeaderMap) -> Option<String> {
    if let Some(forwarded) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first) = forwarded.split(',').next() {
            let trimmed = first.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}
