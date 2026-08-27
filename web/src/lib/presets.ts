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

export const ACCESS_PRESETS: readonly AccessPreset[] = [
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
    id: "facility_ec",
    label: "Facility EC",
    description: "Operational permissions for one facility (no admin tools).",
    scope: "facility",
    roles: ["EC"],
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
