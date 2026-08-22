use std::marker::PhantomData;

use axum::extract::{FromRef, FromRequestParts};
use http::request::Parts;

use crate::{
    auth::{
        acl::PermissionPath,
        context::{CurrentApiKey, CurrentServiceAccount, CurrentUser},
        middleware::ensure_permission,
    },
    errors::ApiError,
    state::AppState,
};

/// A statically-known permission path a route requires. Implemented by marker types
/// generated with [`permission!`] — see `permissions.rs` for the registry.
pub trait Permission {
    fn path() -> PermissionPath;
}

/// Extractor that enforces permission `P` before the handler body runs. Declaring
/// `RequirePermission<P>` in the handler signature is the only way to satisfy it, so
/// a missing check is a visible gap in the signature rather than a silent omission.
pub struct RequirePermission<P: Permission>(PhantomData<P>);

impl<P, S> FromRequestParts<S> for RequirePermission<P>
where
    P: Permission,
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app_state = AppState::from_ref(state);
        let current_user = parts
            .extensions
            .get::<Option<CurrentUser>>()
            .and_then(Option::as_ref);
        let current_service_account = parts
            .extensions
            .get::<Option<CurrentServiceAccount>>()
            .and_then(Option::as_ref);
        let current_api_key = parts
            .extensions
            .get::<Option<CurrentApiKey>>()
            .and_then(Option::as_ref);

        ensure_permission(
            &app_state,
            current_user,
            current_service_account,
            current_api_key,
            P::path(),
        )
        .await?;

        Ok(Self(PhantomData))
    }
}

/// Declares a marker type implementing [`Permission`] for a fixed `PermissionPath`.
macro_rules! permission {
    ($name:ident, [$($segment:literal),+ $(,)?], $action:ident) => {
        pub struct $name;

        impl $crate::auth::require_permission::Permission for $name {
            fn path() -> $crate::auth::acl::PermissionPath {
                $crate::auth::acl::PermissionPath::from_segments(
                    [$($segment),+],
                    $crate::auth::acl::PermissionAction::$action,
                )
            }
        }
    };
}

pub(crate) use permission;
