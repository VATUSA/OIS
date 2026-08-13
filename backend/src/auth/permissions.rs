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

// users directory
permission!(UsersDirectoryRead, ["users", "directory"], Read);

// audit log
permission!(AuditLogsRead, ["audit", "logs"], Read);

// service accounts (bot credential management)
permission!(ServiceAccountsRead, ["service_accounts"], Read);
permission!(ServiceAccountsCreate, ["service_accounts"], Create);
permission!(ServiceAccountsUpdate, ["service_accounts"], Update);
permission!(ServiceAccountsDelete, ["service_accounts"], Delete);

// tmu — traffic management initiatives (TMIs)
permission!(TmuTmiRead, ["tmu", "tmi"], Read);
permission!(TmuTmiCreate, ["tmu", "tmi"], Create);
permission!(TmuTmiUpdate, ["tmu", "tmi"], Update);
permission!(TmuTmiPublish, ["tmu", "tmi"], Publish);
permission!(TmuTmiDelete, ["tmu", "tmi"], Delete);

permission!(TmuProgramRead, ["tmu", "program"], Read);
permission!(TmuProgramUpdate, ["tmu", "program"], Update);
permission!(TmuProgramDelete, ["tmu", "program"], Delete);
