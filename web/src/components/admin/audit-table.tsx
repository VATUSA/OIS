import {DataTable, type DataColumn, type ServerPagination, StatusPill, Tooltip, TooltipContent, TooltipTrigger} from "@ois/ui";
import type {components} from "@ois/api-client";
import {Box, Clock, User, Zap} from "lucide-react";

import {cap, pastTense, resourceLabel, shortId} from "@/lib/audit-format";
import {auditActionTone} from "@/lib/status";
import {formatZuluFull, timeAgo} from "@/lib/time";

type AuditLogEntry = components["schemas"]["AuditLogEntry"];

const COLUMNS: DataColumn<AuditLogEntry>[] = [
  {
    accessorKey: "created_at",
    header: "Time",
    icon: Clock,
    mono: true,
    cell: (c) => (
      <Tooltip>
        <TooltipTrigger asChild>
          <span className="whitespace-nowrap text-ink-2">{formatZuluFull(c.getValue<string>())}</span>
        </TooltipTrigger>
        <TooltipContent>{timeAgo(c.getValue<string>())}</TooltipContent>
      </Tooltip>
    ),
  },
  {
    accessorKey: "action",
    header: "Action",
    icon: Zap,
    cell: (c) => {
      const action = pastTense(c.getValue<string>());
      return <StatusPill tone={auditActionTone(action)}>{cap(action)}</StatusPill>;
    },
  },
  {
    id: "resource",
    accessorFn: (r) => resourceLabel(r.resource_type),
    header: "Resource",
    icon: Box,
    cell: (c) => {
      const id = shortId(c.row.original.resource_id);
      return (
        <span className="whitespace-nowrap">
          {cap(c.getValue<string>())}
          {id && <span className="ml-1.5 font-mono text-xs text-ink-3">{id}</span>}
        </span>
      );
    },
  },
  {
    id: "actor",
    accessorFn: (r) => r.actor_display_name ?? "system",
    header: "Actor",
    icon: User,
    cell: (c) => (
      <span className="whitespace-nowrap">
        <span className="font-semibold">{c.getValue<string>()}</span>
        {c.row.original.actor_cid != null && (
          <span className="ml-1.5 font-mono text-xs text-ink-3">{c.row.original.actor_cid}</span>
        )}
      </span>
    ),
  },
  {
    accessorKey: "reason",
    header: "Reason",
    enableSorting: false,
    cell: (c) => <span className="text-ink-2">{c.getValue<string>() ?? "—"}</span>,
  },
];

/** Audit-log entries. Sorting reorders the loaded page; the API pages newest-first. */
export function AuditTable({
  items,
  isLoading,
  isError,
  serverPagination,
  rowCap,
}: {
  items: readonly AuditLogEntry[];
  isLoading?: boolean;
  isError?: boolean;
  serverPagination?: ServerPagination;
  rowCap?: number;
}) {
  return (
    <DataTable
      label="Audit log"
      columns={COLUMNS}
      data={items}
      getRowId={(r) => r.id}
      initialSort={[{ id: "created_at", desc: true }]}
      serverPagination={serverPagination}
      rowCap={rowCap}
      isLoading={isLoading}
      isError={isError}
      empty="No activity yet."
    />
  );
}
