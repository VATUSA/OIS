/** Shared display helpers for audit-log entries (used by the activity list + the audit table). */

/** Friendly singular noun for each audited resource_type. */
const RESOURCE_LABELS: Record<string, string> = {
  "tmu.tmis": "restriction",
  "tmu.programs": "rate program",
  "tmu.ground-stops": "ground stop",
  "tmu.gdp": "ground delay program",
  "tmu.gdp.slots": "GDP slot",
  "tmu.cfr": "release (CFR)",
  "flow.fcas": "FCA",
  "flow.fcas.release": "FCA release",
  "flow.runway": "runway config",
  "flow.runway.configs": "saved runway config",
  facilities: "facility",
  events: "event",
  "events.dcc": "event DCC",
  "events.facilities": "event facility",
  "events.rates": "event rate",
  "events.staffing": "event staffing request",
  "events.packages": "TMI package",
  "events.packages.items": "package item",
  "admin.service-accounts": "service account",
  // The access editor writes its own richer entry with these exact values.
  USER_ACCESS: "user access",
};

/** Past-tense phrasing for each action verb. */
const PAST_TENSE: Record<string, string> = {
  create: "created",
  update: "updated",
  delete: "deleted",
  publish: "published",
  cancel: "cancelled",
  activate: "activated",
  compress: "compressed",
  rotate: "rotated",
  disable: "disabled",
  refresh: "refreshed",
  reorder: "reordered",
  order: "reordered",
  release: "released",
  roles: "updated roles for",
};

export function cap(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}

export function pastTense(action: string): string {
  const a = action.toLowerCase();
  return PAST_TENSE[a] ?? a;
}

export function resourceLabel(resourceType: string): string {
  return RESOURCE_LABELS[resourceType] ?? resourceType.split(".").pop()!.replace(/[-_]/g, " ");
}

/** Shorten opaque UUIDs to a readable hash; leave meaningful ids (ICAO, callsign) intact. */
export function shortId(id: string | null | undefined): string | null {
  if (!id) return null;
  if (/^[0-9a-f]{8}-[0-9a-f]{4}-/i.test(id)) return `#${id.slice(0, 8)}`;
  return id;
}
