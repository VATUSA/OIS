use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

/// The `/me` response for an authenticated session.
#[derive(Debug, Serialize, ToSchema)]
pub struct MeBody {
    pub id: String,
    pub cid: i64,
    pub email: String,
    pub display_name: String,
    pub rating: Option<String>,
    pub server_admin: bool,
    pub role_names: Vec<String>,
    /// Effective permissions as the nested tree the access editor renders.
    #[schema(value_type = Object)]
    pub permissions: Value,
}

/// A VATUSA facility (ARTCC). `artcc_id` scope values reference `id`.
#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct FacilityBody {
    pub id: String,
    pub name: String,
    pub region: Option<String>,
    pub active: bool,
}

/// Assignable roles + permission catalog + facilities for the access editor.
#[derive(Debug, Serialize, ToSchema)]
pub struct AccessCatalogBody {
    pub roles: Vec<String>,
    /// Every assignable permission as the nested checkbox tree.
    #[schema(value_type = Object)]
    pub permissions: Value,
    /// ARTCCs a grant can be scoped to (drives the scope selector in the editor).
    pub facilities: Vec<FacilityBody>,
}

/// The acting user's own effective access (staff debug view).
#[derive(Debug, Serialize, ToSchema)]
pub struct SelfAccessBody {
    pub server_admin: bool,
    pub role_names: Vec<String>,
    #[schema(value_type = Object)]
    pub permissions: Value,
}

/// A target user's editable access: direct permission grants + role assignments,
/// grouped by scope (national first, then each ARTCC the user has grants/roles in).
#[derive(Debug, Serialize, ToSchema)]
pub struct UserAccessBody {
    pub id: String,
    pub cid: i64,
    pub server_admin: bool,
    pub scopes: Vec<ScopeAccess>,
}

/// Direct grants + roles at one scope. `artcc_id = null` is national.
#[derive(Debug, Serialize, ToSchema)]
pub struct ScopeAccess {
    pub artcc_id: Option<String>,
    pub role_names: Vec<String>,
    /// Direct permission grants at this scope, as the nested checkbox tree.
    #[schema(value_type = Object)]
    pub permissions: Value,
}

/// The editor's SAVE payload. `reason` is required (audited). Each entry in `scopes`
/// replaces that scope's direct permission grants; when its `role_names` is present it
/// also replaces the assignable-role set at that scope. Scopes not listed are untouched.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateUserAccessRequest {
    pub reason: String,
    pub scopes: Vec<ScopeUpdate>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ScopeUpdate {
    /// null / omitted = national scope; otherwise a known ARTCC id.
    #[serde(default)]
    pub artcc_id: Option<String>,
    #[schema(value_type = Object)]
    pub permissions: Value,
    #[serde(default)]
    pub role_names: Option<Vec<String>>,
}

// --- audit log ---

/// One audit-log entry (the "recorded on this controller's log" trail).
#[derive(Debug, Serialize, ToSchema)]
pub struct AuditLogEntry {
    pub id: String,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<String>,
    pub artcc_id: Option<String>,
    pub reason: Option<String>,
    pub actor_cid: Option<i64>,
    pub actor_display_name: Option<String>,
    #[schema(value_type = Option<Object>)]
    pub before_state: Option<Value>,
    #[schema(value_type = Option<Object>)]
    pub after_state: Option<Value>,
    pub created_at: DateTime<Utc>,
}

/// A page of audit-log entries.
#[derive(Debug, Serialize, ToSchema)]
pub struct AuditLogPage {
    pub items: Vec<AuditLogEntry>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

// --- service accounts ---

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateServiceAccountRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetServiceAccountRolesRequest {
    pub role_names: Vec<String>,
}

/// A service account as listed (no secret). `roles` are its granted role names.
#[derive(Debug, Serialize, ToSchema)]
pub struct ServiceAccountBody {
    pub id: String,
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub status: String,
    pub roles: Vec<String>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Returned once on create/rotate — the plaintext bearer token is never stored or
/// shown again.
#[derive(Debug, Serialize, ToSchema)]
pub struct ServiceAccountTokenBody {
    pub account: ServiceAccountBody,
    pub token: String,
}
