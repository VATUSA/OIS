//! A unified authenticated principal — a signed-in user or a user-owned API key — so the
//! scope-enforcing handlers work identically for both. A key's scope is always capped by its
//! owner's live scope, so a key can never reach an ARTCC its owner can't.

use crate::{
    auth::context::{CurrentApiKey, CurrentUser},
    errors::ApiError,
    repos::{
        access::{self as access_repo, PermissionScope},
        api_keys as api_keys_repo,
    },
    state::AppState,
};

/// The actor behind a request for authorization purposes.
#[derive(Debug, Clone)]
pub enum Principal {
    User(CurrentUser),
    ApiKey(CurrentApiKey),
}

impl Principal {
    /// Build from the request's resolved principals (a request carries at most one). Prefers a
    /// session user over a bearer key; returns `Unauthorized` if neither is present.
    pub fn require(
        user: Option<&CurrentUser>,
        api_key: Option<&CurrentApiKey>,
    ) -> Result<Self, ApiError> {
        if let Some(user) = user {
            Ok(Principal::User(user.clone()))
        } else if let Some(key) = api_key {
            Ok(Principal::ApiKey(key.clone()))
        } else {
            Err(ApiError::Unauthorized)
        }
    }

    /// Same as [`Principal::require`] but yields `None` instead of an error when unauthenticated —
    /// for endpoints (e.g. public reads) that resolve an optional caller.
    pub fn optional(user: Option<&CurrentUser>, api_key: Option<&CurrentApiKey>) -> Option<Self> {
        Self::require(user, api_key).ok()
    }

    /// The owning user's id — the user for a session, the key's owner for a key. Use this for
    /// "who acted" attribution (`updated_by`) and as the base for scope resolution.
    pub fn user_id(&self) -> &str {
        match self {
            Principal::User(u) => &u.id,
            Principal::ApiKey(k) => &k.owner_user_id,
        }
    }

    /// This principal's effective scope for `permission_name`. For a user it is their own scope; for
    /// a key it is the owner's scope intersected with the key's granted scope (least privilege).
    pub async fn permission_scope(
        &self,
        state: &AppState,
        permission_name: &str,
    ) -> Result<PermissionScope, ApiError> {
        let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
        match self {
            Principal::User(u) => access_repo::permission_scope(pool, &u.id, permission_name).await,
            Principal::ApiKey(k) => {
                let owner_scope =
                    access_repo::permission_scope(pool, &k.owner_user_id, permission_name).await?;
                let key_scope =
                    api_keys_repo::key_granted_scope(pool, &k.id, permission_name).await?;
                Ok(owner_scope.intersect(&key_scope))
            }
        }
    }
}
