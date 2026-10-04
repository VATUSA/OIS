import {DataTable, type DataColumn, type ServerPagination, Tooltip, TooltipContent, TooltipTrigger} from "@ois/ui";
import type {components} from "@ois/api-client";
import {AppWindow, Clock, Monitor, User} from "lucide-react";

import {formatZuluFull, timeAgo} from "@/lib/time";

type Summary = components["schemas"]["DiagnosticsReportSummary"];

const COLUMNS: DataColumn<Summary>[] = [
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
    id: "user",
    accessorFn: (r) => r.user_display_name,
    header: "Sent by",
    icon: User,
    cell: (c) => (
      <span className="whitespace-nowrap">
        <span className="font-semibold">{c.getValue<string>()}</span>
        <span className="ml-1.5 font-mono text-xs text-ink-3">{c.row.original.user_cid}</span>
        {c.row.original.user_artcc && (
          <span className="ml-1.5 text-xs text-ink-3">{c.row.original.user_artcc}</span>
        )}
      </span>
    ),
  },
  {
    id: "platform",
    accessorFn: (r) => `${r.os} ${r.os_version}`.trim(),
    header: "Platform",
    icon: Monitor,
    cell: (c) => (
      <span className="whitespace-nowrap text-ink-2">
        {c.getValue<string>() || "—"}
        <span className="ml-1.5 font-mono text-xs text-ink-3">v{c.row.original.app_version}</span>
      </span>
    ),
  },
  {
    id: "where",
    accessorFn: (r) => r.route,
    header: "Where",
    icon: AppWindow,
    enableSorting: false,
    cell: (c) => (
      <span className="font-mono text-xs text-ink-2">
        {c.getValue<string>() || "—"}
        {c.row.original.window_label && c.row.original.window_label !== "main" && (
          <span className="ml-1.5 text-ink-3">({c.row.original.window_label})</span>
        )}
      </span>
    ),
  },
  {
    accessorKey: "has_note",
    header: "Note",
    enableSorting: false,
    cell: (c) => <span className="text-ink-2">{c.getValue<boolean>() ? "Yes" : "—"}</span>,
  },
];

/** Diagnostics reports, newest first. Clicking a row opens it. */
export function DiagnosticsTable({
  items,
  isLoading,
  isError,
  serverPagination,
  rowCap,
  onOpen,
}: {
  items: readonly Summary[];
  isLoading?: boolean;
  isError?: boolean;
  serverPagination?: ServerPagination;
  rowCap?: number;
  onOpen: (id: string) => void;
}) {
  return (
    <DataTable
      label="Diagnostics reports"
      columns={COLUMNS}
      data={items}
      getRowId={(r) => r.id}
      initialSort={[{ id: "created_at", desc: true }]}
      serverPagination={serverPagination}
      rowCap={rowCap}
      isLoading={isLoading}
      isError={isError}
      onRowClick={(r) => onOpen(r.id)}
      empty="No reports have been sent."
    />
  );
}
