/**
 * Access-control presets: one-click bundles of permissions (+ a role, for users) applied at a scope.
 * Defined by domain rules against the LIVE catalog so new operational permissions are picked up
 * automatically. Consumed by the admin access editor (users) and the API-key permission picker.
 */

export type PresetScopeKind = "national" | "facility";

export interface AccessPreset {
  id: string;
  label: string;
  description: string;
  /** `national` applies at all-ARTCCs; `facility` applies at one chosen ARTCC. */
  scope: PresetScopeKind;
  /** Roles assigned alongside the permissions. Users only — API keys can't hold roles. */
  roles: string[];
  /** `"all"` = every catalog permission; otherwise an include-list of first-segment domains. */
  domains: "all" | readonly string[];
}

/** Day-to-day operational domains — everything else (access/audit/service_accounts/api_keys/discord/
 *  integration/users/auth) is admin/bot/self and excluded from the facility + DCC presets. */
const OPERATIONAL_DOMAINS = ["tmu", "flow", "events", "ace", "stats"] as const;

/** Traffic-management domains for the NTMO preset. */
const TRAFFIC_DOMAINS = ["tmu", "flow", "stats"] as const;
/** Event-coordination domains for the Events Team preset. */
const EVENT_DOMAINS = ["events", "ace"] as const;

/**
 * The baseline every signed-in user holds via the `USER` role. Presets grant these at NATIONAL scope
 * so the result matches a normal user's defaults — this matters for the facility-EC preset (whose other
 * grants are facility-scoped) and for API keys (which get no roles at all, so no baseline otherwise).
 * Mirror of the `USER` role's `role_permissions`; keep in sync if that role's grants change.
 */
export const BASE_PERMISSIONS = ["ace.requests.create"] as const;

export const ACCESS_PRESETS: readonly AccessPreset[] = [
  // --- National ---
  {
    id: "vatusa_admin",
    label: "VATUSA Admin",
    description: "Every permission, nationally.",
    scope: "national",
    roles: ["VATUSA_STAFF"],
    domains: "all",
  },
  {
    id: "dcc_staff",
    label: "DCC Staff",
    description: "Operational permissions for every ARTCC (no admin tools).",
    scope: "national",
    roles: ["DCC_STAFF"],
    domains: OPERATIONAL_DOMAINS,
  },
  {
    id: "ntmo",
    label: "NTMO",
    description: "National traffic-management tools: TMU, flow, and stats.",
    scope: "national",
    roles: ["NTMO"],
    domains: TRAFFIC_DOMAINS,
  },
  {
    id: "events_team",
    label: "Events Team",
    description: "National event coordination (events + ACE support).",
    scope: "national",
    roles: ["EVENTS_TEAM"],
    domains: EVENT_DOMAINS,
  },
  {
    id: "ace_team",
    label: "ACE Team",
    description: "Handle ACE support requests, nationally.",
    scope: "national",
    roles: ["ACE"],
    domains: ["ace"],
  },
  // --- Facility (scoped to the chosen ARTCC) ---
  {
    id: "facility_ec",
    label: "Facility EC",
    description: "Operational permissions for one facility (no admin tools).",
    scope: "facility",
    roles: ["EC"],
    domains: OPERATIONAL_DOMAINS,
  },
  {
    id: "facility_aec",
    label: "Facility AEC",
    description: "Assistant EC — operational permissions for one facility.",
    scope: "facility",
    roles: ["AEC"],
    domains: OPERATIONAL_DOMAINS,
  },
];

/** The permission names a preset grants, drawn from the given available set (catalog names for users,
 * grantable names for keys — so a preset never exceeds what the actor can actually delegate). */
export function presetPermissions(preset: AccessPreset, available: readonly string[]): string[] {
  if (preset.domains === "all") return [...available];
  const include = new Set(preset.domains);
  return available.filter((p) => include.has(p.split(".")[0]));
}
