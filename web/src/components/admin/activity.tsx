import {Badge} from "@ois/ui";
import type {components} from "@ois/api-client";

import {timeAgo} from "@/lib/time";
import {actionVariant, cap, pastTense, resourceLabel, shortId} from "@/lib/audit-format";

type AuditLogEntry = components["schemas"]["AuditLogEntry"];

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
