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

/// Assignable roles + permission catalog for the access editor.
#[derive(Debug, Serialize)]
pub struct AccessCatalogBody {
    pub roles: Vec<String>,
    /// Every assignable permission as the nested checkbox tree.
    pub permissions: Value,
}

/// The acting user's own effective access (staff debug view).
#[derive(Debug, Serialize)]
pub struct SelfAccessBody {
    pub server_admin: bool,
    pub role_names: Vec<String>,
    pub permissions: Value,
}

/// A target user's access, as read/returned by the editor.
#[derive(Debug, Serialize)]
pub struct UserAccessBody {
    pub id: String,
    pub cid: i64,
    pub server_admin: bool,
    pub role_names: Vec<String>,
    /// Effective permission tree (role-derived + direct grants, minus denies).
    pub permissions: Value,
}

/// The editor's SAVE payload. `reason` is required (audited). `permissions` is the
/// full edited tree; `role_names`, when present, is the complete assignable-role set.
#[derive(Debug, Deserialize)]
pub struct UpdateUserAccessRequest {
    pub reason: String,
    pub permissions: Value,
    #[serde(default)]
    pub role_names: Option<Vec<String>>,
}
