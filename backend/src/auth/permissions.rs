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

// api keys — user-owned personal access tokens. `create` gates the self-service surface (manage
// your OWN keys); `read`/`delete` are oversight over ANY user's keys (held by SERVER_ADMIN implicitly).
permission!(ApiKeysKeyCreate, ["api_keys", "key"], Create);
permission!(ApiKeysKeyRead, ["api_keys", "key"], Read);
permission!(ApiKeysKeyDelete, ["api_keys", "key"], Delete);

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
permission!(EventsConfigUpdate, ["events", "config"], Update);
permission!(EventsSupportUpdate, ["events", "support"], Update);
permission!(EventsDebriefCreate, ["events", "debrief"], Create);
permission!(EventsDiscordPublish, ["events", "discord"], Publish);

// ace — support-request queue + team roster
permission!(AceRequestsRead, ["ace", "requests"], Read);
permission!(AceRequestsCreate, ["ace", "requests"], Create);
permission!(AceRequestsClaim, ["ace", "requests"], Claim);
permission!(AceRequestsDecide, ["ace", "requests"], Decide);
permission!(AceTeamRead, ["ace", "team"], Read);
permission!(AceTeamUpdate, ["ace", "team"], Update);

// discord / integration — the outbound-job queue (bot) + guild config mapping
permission!(IntegrationJobsUpdate, ["integration", "jobs"], Update);
permission!(DiscordConfigRead, ["discord", "config"], Read);
permission!(DiscordConfigUpdate, ["discord", "config"], Update);
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
// Facility map — per-facility aircraft color rules (view is public; editing is facility-scoped).
permission!(FlowFacilityMapUpdate, ["flow", "facility_map"], Update);

// stats — persistent network statistics + saved capture windows
permission!(StatsRead, ["stats", "data"], Read);
permission!(StatsCaptureUpdate, ["stats", "capture"], Update);
