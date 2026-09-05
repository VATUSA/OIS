//! The OIS permission + role catalog (DRAFT — refined per feature spec in
//! `docs/features/*`). This is the national-scale evolution of osmium's catalog:
//! the shared domains carry over, and OIS adds `tmu` (NTML/ADV/TMI), `ace`,
//! `flow` (traffic management), `discord`, and `facilities`.
//!
//! Grants are additionally *scoped* by ARTCC at the assignment layer (a nullable
//! `artcc_id` on `access.user_roles` / `access.user_permissions`; `NULL` = national).
//! Scope is orthogonal to the permission string itself and is enforced in the backend.

pub const SERVER_ADMIN_ROLE: &str = "SERVER_ADMIN";

/// Top-level permission domains — the collapsible groups in the access editor.
pub const DOMAINS: &[&str] = &[
    "access",
    "ace",
    "api_keys",
    "audit",
    "auth",
    "discord",
    "emails",
    "events",
    "facilities",
    "feedback",
    "files",
    "flow",
    "integrations",
    "org",
    "pages",
    "publications",
    "stats",
    "system",
    "system_rate_limit",
    "tmu",
    "training",
    "users",
    "web",
];

/// Starter role set (draft). National roles + facility roles; facility roles are
/// meaningful only in combination with an ARTCC scope on the assignment.
pub fn default_roles() -> Vec<&'static str> {
    vec![
        SERVER_ADMIN_ROLE,
        "USER",
        // Positional / staff roles (assignable; scopable per-ARTCC via artcc_id)
        "VATUSA_STAFF", // division staff
        "EVENTS_TEAM",  // events team
        "EC",           // events coordinator
        "AEC",          // assistant events coordinator
        "ACE",          // ACE team
        "NTMO",         // national traffic management officer
        "DCC_STAFF",    // DCC staff
        // Machine actors
        "BOT",
        "SERVICE_APP",
    ]
}

/// Draft permission catalog for the OIS-specific domains. The shared domains
/// (access/auth/users/events/…) are ported from osmium during Phase 0; the entries
/// below are the new national tooling and are firmed up in their feature specs.
pub fn draft_new_permission_names() -> Vec<&'static str> {
    vec![
        // --- events (operational coordination; posting/review stays in the current
        // VATUSA website — OIS owns the prior/during/post window) ---
        "events.plan.read",                // view an event's planning package
        "events.plan.update",              // edit DCC / facility support / TMI packages
        "events.rate.update",              // set an event's airport AAR/ADR (facility-scoped)
        "events.config.update", // manage an airport's default runway configs (facility-scoped)
        "events.support.update", // set a facility's event support level (facility-scoped)
        "events.staffing_requests.create", // CC an ARTCC / request staffing
        "events.staffing_requests.read",
        "events.staffing_requests.decide", // acknowledge/decline a staffing request
        "events.slots.claim",              // book an open event position slot
        "events.discord.publish",          // open the coordination thread + ping staff
        "events.debrief.read",             // read the post-event debrief
        "events.debrief.create",           // write a post-event debrief entry
        // --- tmu: NTML / ADV / TMI ---
        "tmu.ntml.read",
        "tmu.ntml.create",
        "tmu.ntml.update",
        "tmu.ntml.delete",
        "tmu.adv.read",
        "tmu.adv.create",
        "tmu.adv.update",
        "tmu.adv.publish",
        "tmu.tmi.read",
        "tmu.tmi.create",
        "tmu.tmi.update",
        "tmu.tmi.publish",
        "tmu.tmi.delete",
        "tmu.gdp.read",
        "tmu.gdp.create",
        "tmu.gdp.publish",
        "tmu.gdp.delete",
        "tmu.delays.read",
        // --- ace: support requests + team ---
        "ace.requests.read",
        "ace.requests.create",
        "ace.requests.claim",
        "ace.requests.decide",
        "ace.team.read",
        "ace.team.update",
        // --- flow: traffic management (vatflow-style capabilities, native to OIS) ---
        "flow.programs.read",
        "flow.programs.create",
        "flow.programs.update",
        "flow.programs.publish",
        "flow.programs.delete",
        "flow.data.read",
        "flow.fca.read",
        "flow.fca.update",
        "flow.fca.delete",
        "flow.route.update",
        "flow.route.delete",
        "flow.runway.read",
        "flow.runway.update",
        "flow.facility_map.update", // edit a facility map's aircraft color rules (facility-scoped)
        "flow.aircraft_profiles.read", // view aircraft performance profiles
        "flow.aircraft_profiles.update", // manage aircraft performance profiles (national)
        // --- stats: persistent network statistics + saved capture windows ---
        "stats.data.read",
        "stats.capture.update",
        // --- discord config ---
        "discord.config.read",
        "discord.config.update",
        // --- facilities ---
        "facilities.directory.read",
        "facilities.directory.update",
        // --- api keys: user-owned personal access tokens (bounded by the owner's live access) ---
        "api_keys.key.create", // create and manage your OWN keys
        "api_keys.key.read",   // view any user's keys (oversight)
        "api_keys.key.delete", // revoke any user's keys (oversight)
    ]
}
