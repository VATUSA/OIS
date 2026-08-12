//! Registry of statically-known permission marker types for `RequirePermission<P>`.
//! Grows as handlers are added, one entry per (segments, action) a route enforces.

use crate::auth::require_permission::permission;

// auth (self-service profile + session)
permission!(AuthProfileRead, ["auth", "profile"], Read);
permission!(AuthProfileUpdate, ["auth", "profile"], Update);
permission!(AuthSessionsDelete, ["auth", "sessions"], Delete);

// access editor (the permission-editor UI backend)
permission!(AccessSelfRead, ["access", "self"], Read);
permission!(AccessCatalogRead, ["access", "catalog"], Read);
permission!(AccessUsersRead, ["access", "users"], Read);
permission!(AccessUsersUpdate, ["access", "users"], Update);
