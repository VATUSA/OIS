//! Audit-log read endpoint.

use axum::{
    Json,
    extract::{Query, State},
};
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
    action: Option<String>,
    page: Option<i64>,
    page_size: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/audit",
    tag = "audit",
    params(
        ("resource_type" = Option<String>, Query, description = "Filter by resource type"),
        ("action" = Option<String>, Query, description = "Filter by action"),
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
        resource_type: query
            .resource_type
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        action: query
            .action
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
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
