use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The `/me` response for an authenticated session.
#[derive(Debug, Serialize)]
pub struct MeBody {
    pub id: String,
    pub cid: i64,
    pub email: String,
    pub display_name: String,
    pub rating: Option<String>,
    pub server_admin: bool,
    pub role_names: Vec<String>,
    /// Effective permissions as the nested tree the access editor renders.
    pub permissions: Value,
}

/// A VATUSA facility (ARTCC). `artcc_id` scope values reference `id`.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct FacilityBody {
    pub id: String,
    pub name: String,
    pub region: Option<String>,
    pub active: bool,
}

/// Assignable roles + permission catalog + facilities for the access editor.
#[derive(Debug, Serialize)]
pub struct AccessCatalogBody {
    pub roles: Vec<String>,
    /// Every assignable permission as the nested checkbox tree.
    pub permissions: Value,
    /// ARTCCs a grant can be scoped to (drives the scope selector in the editor).
    pub facilities: Vec<FacilityBody>,
}

/// The acting user's own effective access (staff debug view).
#[derive(Debug, Serialize)]
pub struct SelfAccessBody {
    pub server_admin: bool,
    pub role_names: Vec<String>,
    pub permissions: Value,
}

/// A target user's editable access: direct permission grants + role assignments,
/// grouped by scope (national first, then each ARTCC the user has grants/roles in).
#[derive(Debug, Serialize)]
pub struct UserAccessBody {
    pub id: String,
    pub cid: i64,
    pub server_admin: bool,
    pub scopes: Vec<ScopeAccess>,
}

/// Direct grants + roles at one scope. `artcc_id = null` is national.
#[derive(Debug, Serialize)]
pub struct ScopeAccess {
    pub artcc_id: Option<String>,
    pub role_names: Vec<String>,
    /// Direct permission grants at this scope, as the nested checkbox tree.
    pub permissions: Value,
}

/// The editor's SAVE payload. `reason` is required (audited). Each entry in `scopes`
/// replaces that scope's direct permission grants; when its `role_names` is present it
/// also replaces the assignable-role set at that scope. Scopes not listed are untouched.
#[derive(Debug, Deserialize)]
pub struct UpdateUserAccessRequest {
    pub reason: String,
    pub scopes: Vec<ScopeUpdate>,
}

#[derive(Debug, Deserialize)]
pub struct ScopeUpdate {
    /// null / omitted = national scope; otherwise a known ARTCC id.
    #[serde(default)]
    pub artcc_id: Option<String>,
    pub permissions: Value,
    #[serde(default)]
    pub role_names: Option<Vec<String>>,
}
