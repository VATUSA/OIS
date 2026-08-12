use serde::{Deserialize, Serialize};

/// The resolved current user for a request, populated by `resolve_current_user`.
/// Trimmed from osmium (no impersonation/profile yet).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CurrentUser {
    pub id: String,
    pub cid: i64,
    pub email: String,
    pub display_name: String,
    pub rating: Option<String>,
    pub primary_role: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CurrentServiceAccount {
    pub id: String,
    pub key: String,
    pub name: String,
}

/// Newtype wrapper for the session cookie value so it can live in request extensions
/// without colliding with the bearer token (both are `Option<String>`, and axum keys
/// extensions by type).
#[derive(Debug, Clone)]
pub struct SessionToken(pub Option<String>);
