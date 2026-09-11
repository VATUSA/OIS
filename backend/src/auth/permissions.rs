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

// Background jobs viewer (issue #34): read statuses, and "update" = trigger an immediate run.
permission!(SystemJobsRead, ["system", "jobs"], Read);
permission!(SystemJobsUpdate, ["system", "jobs"], Update);

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
permission!(EventsAvailabilityUpdate, ["events", "availability"], Update);

// ace — event-scoped support-request queue
permission!(AceRequestsRead, ["ace", "requests"], Read);
permission!(AceRequestsCreate, ["ace", "requests"], Create);
permission!(AceRequestsClaim, ["ace", "requests"], Claim);
permission!(AceRequestsDecide, ["ace", "requests"], Decide);

// discord / integration — the outbound-job queue (bot) + guild config mapping
permission!(IntegrationJobsUpdate, ["integration", "jobs"], Update);
permission!(DiscordConfigRead, ["discord", "config"], Read);
permission!(DiscordConfigUpdate, ["discord", "config"], Update);

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
// Aircraft performance profiles for the trajectory/ETA model (national reference data).
permission!(
    FlowAircraftProfilesRead,
    ["flow", "aircraft_profiles"],
    Read
);
permission!(
    FlowAircraftProfilesUpdate,
    ["flow", "aircraft_profiles"],
    Update
);

// stats — persistent network statistics + saved capture windows
permission!(StatsRead, ["stats", "data"], Read);
permission!(StatsCaptureUpdate, ["stats", "capture"], Update);

#[cfg(test)]
mod sync_tests {
    use std::collections::HashSet;

    use sqlx::PgPool;

    /// Every `permission!` macro invocation's derived dotted name (`segments.joined.action`),
    /// parsed directly from this file's own source — the only way to enumerate every marker
    /// without a compile-time registry (no `inventory`/`linkme` dependency exists in this
    /// workspace, and source-parsing is the standard cheap-test pattern for this). Note: avoid
    /// writing the macro-call text `permission!` immediately followed by an open paren anywhere
    /// in this module's own comments/strings, or the scan below will find and misparse it too.
    fn parse_marker_names() -> Vec<String> {
        let source = include_str!("permissions.rs");
        // Split across two literals so this very source line doesn't match itself when the whole
        // file (this test module included) gets scanned below.
        let needle = concat!("permission", "!(");
        let mut names = Vec::new();
        let mut rest = source;
        while let Some(start) = rest.find(needle) {
            rest = &rest[start + needle.len()..];
            let end = rest.find(')').expect("unterminated permission! invocation");
            let args = &rest[..end];
            rest = &rest[end + 1..];

            // `args` is `Name, ["seg1", "seg2"], Action` (whitespace/newlines allowed anywhere).
            let bracket_start = args.find('[').expect("permission! missing segment list");
            let bracket_end = args.find(']').expect("permission! missing segment list");
            let segments: Vec<&str> = args[bracket_start + 1..bracket_end]
                .split(',')
                .map(|s| s.trim().trim_matches('"'))
                .filter(|s| !s.is_empty())
                .collect();
            let action = args[bracket_end + 1..].trim_start_matches(',').trim();
            names.push(format!(
                "{}.{}",
                segments.join("."),
                action.to_ascii_lowercase()
            ));
        }
        names
    }

    /// A `permission!` marker is meant to be grantable and documented, not just enforced — if it
    /// only exists here, the access editor can never show or grant it, and the catalog silently
    /// drifts from what the code actually checks (see #72/#74's `events_repo::list_all` incident
    /// for what unaudited drift like this costs). Directional on purpose: catalog/migration
    /// entries with *no* marker (`tmu.ntml.*`, `ace.team.*`, etc.) are already documented,
    /// intentional not-yet-implemented state (see docs/features/*.md) — this only catches the
    /// failure mode the issue describes, forgetting one of the other two places for something
    /// that's actually enforced.
    #[sqlx::test]
    async fn every_permission_marker_has_a_catalog_entry_and_a_migration_row(pool: PgPool) {
        let markers = parse_marker_names();
        assert!(
            markers.len() > 50,
            "sanity: the source parser should find every permission! invocation in this file"
        );

        let catalog: HashSet<&str> = ois_core::catalog::draft_new_permission_names()
            .into_iter()
            .collect();
        let db_rows: HashSet<String> = sqlx::query_scalar("select name from access.permissions")
            .fetch_all(&pool)
            .await
            .unwrap()
            .into_iter()
            .collect();

        for name in &markers {
            assert!(
                catalog.contains(name.as_str()),
                "{name}: has a permission! marker in permissions.rs but no entry in \
                 ois_core::catalog::draft_new_permission_names() — add it there"
            );
            assert!(
                db_rows.contains(name),
                "{name}: has a permission! marker in permissions.rs but no \
                 `insert into access.permissions` row in any migration — add one"
            );
        }
    }

    /// Same directional check for the role trio: every role the access editor can actually grant
    /// (`ASSIGNABLE_USER_ROLES`) must be a real default role and exist in the DB. Not the reverse
    /// — `default_roles()` includes non-assignable machine/bootstrapped roles (`SERVER_ADMIN`,
    /// `BOT`, `SERVICE_APP`) by design, and migration 0004's superseded role names are separate
    /// pre-existing cruft, not this check's concern.
    #[sqlx::test]
    async fn every_assignable_role_has_a_default_and_a_migration_row(pool: PgPool) {
        let defaults: HashSet<&str> = ois_core::catalog::default_roles().into_iter().collect();
        let db_roles: HashSet<String> = sqlx::query_scalar("select name from access.roles")
            .fetch_all(&pool)
            .await
            .unwrap()
            .into_iter()
            .collect();

        for role in crate::repos::access::ASSIGNABLE_USER_ROLES {
            assert!(
                defaults.contains(role),
                "{role}: in ASSIGNABLE_USER_ROLES but not ois_core::catalog::default_roles()"
            );
            assert!(
                db_roles.contains(*role),
                "{role}: in ASSIGNABLE_USER_ROLES but no `insert into access.roles` row in any \
                 migration"
            );
        }
    }
}
