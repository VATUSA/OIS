//! A unified authenticated principal — a signed-in user, a user-owned API key, or a service
//! account — so the scope-enforcing handlers work identically for all of them. A key's scope is
//! always capped by its owner's live scope, so a key can never reach an ARTCC its owner can't.
//!
//! A handler takes [`Actor`] (via [`Principal::require_any`]), which admits all three (#583, #607);
//! `Option<Actor>` for a public route that only reads the caller.

use crate::{
    auth::context::{CurrentApiKey, CurrentServiceAccount, CurrentUser},
    errors::ApiError,
    repos::{
        access::{self as access_repo, PermissionScope},
        api_keys as api_keys_repo, audit as audit_repo,
    },
    state::AppState,
};

/// The actor behind a request for authorization purposes.
#[derive(Debug, Clone)]
pub enum Principal {
    User(CurrentUser),
    ApiKey(CurrentApiKey),
    /// Only ever built by [`Principal::require_any`]; [`Principal::require`] never yields one.
    ServiceAccount(CurrentServiceAccount),
}

/// Who a write is attributed to (#583). `user_id` fills the legacy `*_by` column (a foreign key to
/// `identity.users`) and is set only for a signed-in user; `actor_id` fills the `*_by_actor` column
/// (a foreign key to `access.actors`) for every principal, so a machine's write names the machine —
/// never a person, and never nobody.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attribution {
    pub user_id: Option<String>,
    pub actor_id: Option<String>,
}

impl Attribution {
    /// For a machine's write (no user — a service account or a key), the actor that must already hold
    /// the row for a conditional write to touch it (#585). People win, so a machine may only ever
    /// replace, swap or clear its **own** release, and that has to be part of the write itself rather
    /// than a read before it. `None` for a person, whose writes are unconditional. A machine with no
    /// resolved actor gets `""`, which matches no row: fail closed.
    pub fn machine_actor(&self) -> Option<&str> {
        match self.user_id {
            Some(_) => None,
            None => Some(self.actor_id.as_deref().unwrap_or("")),
        }
    }

    /// A signed-in user's attribution, from their id alone — for writes that happen outside a request,
    /// where there is no [`Principal`]: the event-package lifecycle job runs a package's TMIs and
    /// advisories as the person who scheduled it. Resolves their actor exactly as
    /// [`Principal::attribution`] does, so the row is attributed in both columns.
    pub async fn for_user_id(pool: &sqlx::PgPool, user_id: &str) -> Result<Self, ApiError> {
        let display_name: String =
            sqlx::query_scalar("select display_name from identity.users where id = $1")
                .bind(user_id)
                .fetch_optional(pool)
                .await
                .map_err(|_| ApiError::Internal)?
                .ok_or(ApiError::Internal)?;
        Ok(Self {
            user_id: Some(user_id.to_string()),
            actor_id: audit_repo::resolve_user_actor_id(pool, user_id, &display_name).await?,
        })
    }
}

#[cfg(test)]
impl Principal {
    /// A signed-in user's principal from their id — for tests that drive `activate_package` directly.
    pub async fn for_user_id(pool: &sqlx::PgPool, user_id: &str) -> Self {
        Principal::User(
            sqlx::query_as::<_, CurrentUser>(
                "select id, coalesce(cid, 0) as cid, coalesce(email::text, '') as email, display_name, rating, \
                 null::text as primary_role from identity.users where id = $1",
            )
            .bind(user_id)
            .fetch_one(pool)
            .await
            .expect("a seeded user"),
        )
    }
}

#[cfg(test)]
impl Attribution {
    /// A user's attribution without resolving an actor — for repo-level tests only. Production code
    /// goes through [`Principal::attribution`], which always names the actor.
    pub fn user_only(user_id: &str) -> Self {
        Self {
            user_id: Some(user_id.to_string()),
            actor_id: None,
        }
    }
}

impl Principal {
    /// Build from the request's resolved principals (a request carries at most one). Prefers a
    /// session user over a bearer key; returns `Unauthorized` if neither is present.
    fn require(
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

    /// Like [`Principal::require`], but also admits a service account. Use it (through [`Actor`]) only
    /// in a handler whose every use of the principal handles the `ServiceAccount` arm.
    pub fn require_any(
        user: Option<&CurrentUser>,
        service_account: Option<&CurrentServiceAccount>,
        api_key: Option<&CurrentApiKey>,
    ) -> Result<Self, ApiError> {
        match service_account {
            Some(sa) if user.is_none() => Ok(Principal::ServiceAccount(sa.clone())),
            _ => Self::require(user, api_key),
        }
    }

    /// The owning user's id — the user for a session, the key's owner for a key, and `None` for a
    /// service account, which belongs to no user. Use this for "who acted" attribution (`updated_by`)
    /// where the column is a user foreign key; a handler that admits service accounts uses
    /// [`Principal::attribution`] instead.
    pub fn user_id(&self) -> Option<&str> {
        match self {
            Principal::User(u) => Some(&u.id),
            Principal::ApiKey(k) => Some(&k.owner_user_id),
            Principal::ServiceAccount(_) => None,
        }
    }

    /// The attribution for a row this principal writes. A user is named in both columns; a key or a
    /// service account only by its own actor, so the row says a machine did it (#583 AC2). A key's
    /// owner stays reachable through `access.actors.api_key_id`.
    pub async fn attribution(&self, state: &AppState) -> Result<Attribution, ApiError> {
        self.attribution_in(state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?)
            .await
    }

    /// [`Principal::attribution`] for work outside a request, which has a pool but no `AppState`.
    pub async fn attribution_in(&self, pool: &sqlx::PgPool) -> Result<Attribution, ApiError> {
        Ok(match self {
            Principal::User(u) => Attribution {
                user_id: Some(u.id.clone()),
                actor_id: audit_repo::resolve_user_actor_id(pool, &u.id, &u.display_name).await?,
            },
            Principal::ApiKey(k) => Attribution {
                user_id: None,
                actor_id: audit_repo::resolve_api_key_actor_id(
                    pool,
                    &k.id,
                    &format!("{} ({})", k.name, k.prefix),
                )
                .await?,
            },
            Principal::ServiceAccount(sa) => Attribution {
                user_id: None,
                actor_id: audit_repo::resolve_service_account_actor_id(pool, &sa.id, &sa.name)
                    .await?,
            },
        })
    }

    /// This principal's effective scope for `permission_name`. For a user it is their own scope; for
    /// a key it is the owner's scope intersected with the key's granted scope (least privilege).
    pub async fn permission_scope(
        &self,
        state: &AppState,
        permission_name: &str,
    ) -> Result<PermissionScope, ApiError> {
        self.permission_scope_in(
            state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?,
            permission_name,
        )
        .await
    }

    /// [`Principal::permission_scope`] for work outside a request.
    pub async fn permission_scope_in(
        &self,
        pool: &sqlx::PgPool,
        permission_name: &str,
    ) -> Result<PermissionScope, ApiError> {
        match self {
            Principal::User(u) => access_repo::permission_scope(pool, &u.id, permission_name).await,
            Principal::ApiKey(k) => {
                let owner_scope =
                    access_repo::permission_scope(pool, &k.owner_user_id, permission_name).await?;
                let key_scope =
                    api_keys_repo::key_granted_scope(pool, &k.id, permission_name).await?;
                Ok(owner_scope.intersect(&key_scope))
            }
            // Its roles carry an ARTCC, so a ZDC-scoped account is not national (#583 AC5).
            Principal::ServiceAccount(sa) => {
                access_repo::service_account_permission_scope(pool, &sa.id, permission_name).await
            }
        }
    }

    /// Rebuild the principal an `access.actors` row names, for work done later on its behalf — the
    /// event-package lifecycle job publishes a package as whoever armed it (#607). `None` when that
    /// credential no longer works: a revoked or expired key, a disabled service account, a deleted
    /// user or key. Nothing then acts as it, exactly as a request with that credential would be refused.
    pub async fn from_actor(pool: &sqlx::PgPool, actor_id: &str) -> Result<Option<Self>, ApiError> {
        if let Some(u) = access_repo::find_current_user_by_actor(pool, actor_id).await? {
            return Ok(Some(Principal::User(u)));
        }
        if let Some(k) = access_repo::find_current_api_key_by_actor(pool, actor_id).await? {
            return Ok(Some(Principal::ApiKey(k)));
        }
        Ok(
            access_repo::find_current_service_account_by_actor(pool, actor_id)
                .await?
                .map(Principal::ServiceAccount),
        )
    }
}

/// The extractor for a handler that any credential may drive (#583): a session user, a user's API key,
/// or a service account. It replaces `Extension<Option<CurrentUser>>` + `ok_or(Unauthorized)`, which
/// let a machine through `RequirePermission` and then refused it on the next line.
pub struct Actor(pub Principal);

impl<S> axum::extract::FromRequestParts<S> for Actor
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let ext = &parts.extensions;
        Principal::require_any(
            ext.get::<Option<CurrentUser>>().and_then(Option::as_ref),
            ext.get::<Option<CurrentServiceAccount>>()
                .and_then(Option::as_ref),
            ext.get::<Option<CurrentApiKey>>().and_then(Option::as_ref),
        )
        .map(Actor)
    }
}

/// `Option<Actor>`, for a public route that only reads the caller to decide what it may edit.
impl<S> axum::extract::OptionalFromRequestParts<S> for Actor
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> Result<Option<Self>, Self::Rejection> {
        Ok(
            <Actor as axum::extract::FromRequestParts<S>>::from_request_parts(parts, state)
                .await
                .ok(),
        )
    }
}
