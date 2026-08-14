import {Badge} from "@ois/ui";
import type {components} from "@ois/api-client";

import {timeAgo} from "@/lib/time";

type AuditLogEntry = components["schemas"]["AuditLogEntry"];

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
  "facilities": "facility",
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

function cap(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}

function pastTense(action: string): string {
  const a = action.toLowerCase();
  return PAST_TENSE[a] ?? a;
}

function resourceLabel(resourceType: string): string {
  return (
    RESOURCE_LABELS[resourceType] ??
    resourceType.split(".").pop()!.replace(/[-_]/g, " ")
  );
}

/** Shorten opaque UUIDs to a readable hash; leave meaningful ids (ICAO, callsign) intact. */
function shortId(id: string | null | undefined): string | null {
  if (!id) return null;
  if (/^[0-9a-f]{8}-[0-9a-f]{4}-/i.test(id)) return `#${id.slice(0, 8)}`;
  return id;
}

function actionVariant(action: string): "success" | "destructive" | "secondary" {
  const a = action.toLowerCase();
  if (/(create|assign|grant|add|publish|activat)/.test(a)) return "success";
  if (/(delete|revoke|disable|remove|deny|cancel)/.test(a)) return "destructive";
  return "secondary";
}

export function ActivityList({ items }: { items: AuditLogEntry[] }) {
  if (items.length === 0) {
    return (
      <p className="py-6 text-center text-sm text-muted-foreground">
        No activity yet.
      </p>
    );
  }
  return (
    <ul className="divide-y">
      {items.map((entry) => {
        const action = pastTense(entry.action);
        const resource = resourceLabel(entry.resource_type);
        const id = shortId(entry.resource_id);
        return (
          <li key={entry.id} className="flex items-start gap-3 py-3">
            <Badge
              variant={actionVariant(action)}
              className="mt-0.5 shrink-0"
            >
              {cap(action)}
            </Badge>
            <div className="min-w-0 flex-1">
              <p className="truncate text-sm">
                {cap(resource)}
                {id ? (
                  <span className="ml-1 font-mono text-xs text-muted-foreground">
                    {id}
                  </span>
                ) : null}
                {entry.reason ? (
                  <span className="text-muted-foreground"> — {entry.reason}</span>
                ) : null}
              </p>
              <p className="text-xs text-muted-foreground">
                {entry.actor_display_name ?? "system"} · {timeAgo(entry.created_at)}
              </p>
            </div>
          </li>
        );
      })}
    </ul>
  );
}
