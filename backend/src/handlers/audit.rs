//! Audit-log read endpoint.

use axum::{
    Json,
    extract::{Query, State},
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::{
    auth::{permissions::AuditLogsRead, require_permission::RequirePermission},
    errors::ApiError,
    models::AuditLogPage,
    repos::audit as audit_repo,
    state::AppState,
};

#[derive(Deserialize)]
pub struct AuditListQuery {
    resource_type: Option<String>,
    resource_id: Option<String>,
    action: Option<String>,
    actor_id: Option<String>,
    /// Free-text search across action / resource / reason / actor name + CID.
    q: Option<String>,
    /// Inclusive created_at range (RFC 3339).
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    page: Option<i64>,
    page_size: Option<i64>,
}

/// Trim to a non-empty value, or `None`.
fn clean(value: Option<&String>) -> Option<String> {
    value
        .map(String::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/audit",
    tag = "audit",
    params(
        ("resource_type" = Option<String>, Query, description = "Filter by resource type"),
        ("resource_id" = Option<String>, Query, description = "Filter by resource id (the acted-on target)"),
        ("action" = Option<String>, Query, description = "Filter by action"),
        ("actor_id" = Option<String>, Query, description = "Filter to one actor (per-actor dossier)"),
        ("q" = Option<String>, Query, description = "Free-text search (action / resource / reason / actor)"),
        ("from" = Option<String>, Query, description = "Only entries at/after this time (RFC 3339)"),
        ("to" = Option<String>, Query, description = "Only entries at/before this time (RFC 3339)"),
        ("page" = Option<i64>, Query, description = "1-based page (default 1)"),
        ("page_size" = Option<i64>, Query, description = "Page size (default 50, max 100)")
    ),
    responses((status = 200, body = AuditLogPage), (status = 401))
)]
pub async fn list_audit_logs(
    State(state): State<AppState>,
    _permission: RequirePermission<AuditLogsRead>,
    Query(query): Query<AuditListQuery>,
) -> Result<Json<AuditLogPage>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(50).clamp(1, 100);
    let filters = audit_repo::AuditLogFilters {
        resource_type: clean(query.resource_type.as_ref()),
        resource_id: clean(query.resource_id.as_ref()),
        action: clean(query.action.as_ref()),
        actor_id: clean(query.actor_id.as_ref()),
        search: clean(query.q.as_ref()),
        from: query.from,
        to: query.to,
        limit: page_size,
        offset: (page - 1) * page_size,
    };

    let total = audit_repo::count_audit_logs(pool, &filters).await?;
    let items = audit_repo::fetch_audit_logs(pool, &filters).await?;

    Ok(Json(AuditLogPage {
        items,
        total,
        page,
        page_size,
    }))
}
