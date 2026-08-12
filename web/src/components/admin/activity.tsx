import {Badge} from "@ois/ui";
import type {components} from "@ois/api-client";

import {timeAgo} from "@/lib/time";

type AuditLogEntry = components["schemas"]["AuditLogEntry"];

function actionVariant(
  action: string,
): "success" | "destructive" | "secondary" {
  const a = action.toLowerCase();
  if (/(create|assign|grant|add|publish)/.test(a)) return "success";
  if (/(delete|revoke|disable|remove|deny)/.test(a)) return "destructive";
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
      {items.map((entry) => (
        <li key={entry.id} className="flex items-start gap-3 py-3">
          <Badge
            variant={actionVariant(entry.action)}
            className="mt-0.5 shrink-0"
          >
            {entry.action}
          </Badge>
          <div className="min-w-0 flex-1">
            <p className="truncate text-sm">
              <span className="text-muted-foreground">
                {entry.resource_type}
              </span>
              {entry.reason ? ` — ${entry.reason}` : ""}
            </p>
            <p className="text-xs text-muted-foreground">
              {entry.actor_display_name ?? "system"} ·{" "}
              {timeAgo(entry.created_at)}
            </p>
          </div>
        </li>
      ))}
    </ul>
  );
}
