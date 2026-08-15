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

permission!(TmuGroundStopRead, ["tmu", "groundstop"], Read);
permission!(TmuGroundStopCreate, ["tmu", "groundstop"], Create);
permission!(TmuGroundStopPublish, ["tmu", "groundstop"], Publish);
permission!(TmuGroundStopDelete, ["tmu", "groundstop"], Delete);

permission!(TmuGdpRead, ["tmu", "gdp"], Read);
permission!(TmuGdpCreate, ["tmu", "gdp"], Create);
permission!(TmuGdpPublish, ["tmu", "gdp"], Publish);
permission!(TmuGdpDelete, ["tmu", "gdp"], Delete);

permission!(TmuCfrAssign, ["tmu", "cfr"], Assign);

// events — per-event planning
permission!(EventsPlanRead, ["events", "plan"], Read);
permission!(EventsPlanUpdate, ["events", "plan"], Update);
permission!(EventsRateUpdate, ["events", "rate"], Update);
permission!(
    EventsStaffingCreate,
    ["events", "staffing_requests"],
    Create
);

// flow — flow constrained areas (FCAs)
permission!(FlowFcaRead, ["flow", "fca"], Read);
permission!(FlowFcaUpdate, ["flow", "fca"], Update);
permission!(FlowFcaDelete, ["flow", "fca"], Delete);
// Routes are visible to anyone who can view the flow map (FlowFcaRead); editing/deleting
// them needs these dedicated perms.
permission!(FlowRouteUpdate, ["flow", "route"], Update);
permission!(FlowRouteDelete, ["flow", "route"], Delete);
permission!(FlowRunwayRead, ["flow", "runway"], Read);
permission!(FlowRunwayUpdate, ["flow", "runway"], Update);
