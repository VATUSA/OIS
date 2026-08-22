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

/// A resolved user-owned API key (personal access token), populated from an
/// `Authorization: Bearer ois_pat_…` header. Its authority is capped at request time by the
/// owner's live permissions — see [`crate::auth::principal`].
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CurrentApiKey {
    pub id: String,
    pub owner_user_id: String,
    pub prefix: String,
    pub name: String,
}

/// Newtype wrapper for the session cookie value so it can live in request extensions
/// without colliding with the bearer token (both are `Option<String>`, and axum keys
/// extensions by type).
#[derive(Debug, Clone)]
pub struct SessionToken(pub Option<String>);
