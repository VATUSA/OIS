import {useMemo, useState} from "react";
import {Badge, Tooltip, TooltipContent, TooltipTrigger} from "@ois/ui";
import type {components} from "@ois/api-client";
import {type SortingState, flexRender} from "@tanstack/react-table";
import {
  getCoreRowModel,
  getSortedRowModel,
  type LegacyColumnDef as ColumnDef,
  useLegacyTable as useReactTable,
} from "@tanstack/react-table/legacy";
import {ArrowDown, ArrowUp, ChevronsUpDown} from "lucide-react";

import {formatZuluFull, timeAgo} from "@/lib/time";
import {actionVariant, cap, pastTense, resourceLabel, shortId} from "@/lib/audit-format";

type AuditLogEntry = components["schemas"]["AuditLogEntry"];

/** The audit log as a sortable TanStack table. Sorting reorders the current page (the list is
 * offset-paginated server-side, newest first). */
export function AuditTable({ items }: { items: AuditLogEntry[] }) {
  const [sorting, setSorting] = useState<SortingState>([{ id: "created_at", desc: true }]);

  const columns = useMemo<ColumnDef<AuditLogEntry>[]>(
    () => [
      {
        accessorKey: "created_at",
        header: "Time",
        cell: (c) => (
          <Tooltip>
            <TooltipTrigger asChild>
              <span className="whitespace-nowrap font-mono text-xs text-muted-foreground">
                {formatZuluFull(c.getValue<string>())}
              </span>
            </TooltipTrigger>
            <TooltipContent>{timeAgo(c.getValue<string>())}</TooltipContent>
          </Tooltip>
        ),
      },
      {
        accessorKey: "action",
        header: "Action",
        cell: (c) => {
          const action = pastTense(c.getValue<string>());
          return <Badge variant={actionVariant(action)}>{cap(action)}</Badge>;
        },
      },
      {
        id: "resource",
        accessorFn: (r) => resourceLabel(r.resource_type),
        header: "Resource",
        cell: (c) => {
          const id = shortId(c.row.original.resource_id);
          return (
            <span className="whitespace-nowrap">
              {cap(c.getValue<string>())}
              {id && <span className="ml-1.5 font-mono text-xs text-muted-foreground">{id}</span>}
            </span>
          );
        },
      },
      {
        id: "actor",
        accessorFn: (r) => r.actor_display_name ?? "system",
        header: "Actor",
        cell: (c) => (
          <span className="whitespace-nowrap">
            {c.getValue<string>()}
            {c.row.original.actor_cid != null && (
              <span className="ml-1.5 text-xs text-muted-foreground">{c.row.original.actor_cid}</span>
            )}
          </span>
        ),
      },
      {
        accessorKey: "reason",
        header: "Reason",
        enableSorting: false,
        cell: (c) => <span className="text-muted-foreground">{c.getValue<string>() ?? "—"}</span>,
      },
    ],
    [],
  );

  const table = useReactTable({
    data: items,
    columns,
    state: { sorting },
    onSortingChange: setSorting,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
  });

  if (items.length === 0) {
    return <p className="py-6 text-center text-sm text-muted-foreground">No activity yet.</p>;
  }

  return (
    <div className="overflow-x-auto">
      <table className="w-full text-sm">
        <thead>
          {table.getHeaderGroups().map((hg) => (
            <tr key={hg.id} className="border-b text-left text-xs uppercase tracking-wide text-muted-foreground">
              {hg.headers.map((h) => {
                const sorted = h.column.getIsSorted();
                const sortable = h.column.getCanSort();
                return (
                  <th
                    key={h.id}
                    className={"whitespace-nowrap py-2 pr-4 font-medium " + (sortable ? "cursor-pointer select-none" : "")}
                    onClick={sortable ? h.column.getToggleSortingHandler() : undefined}
                  >
                    <span className="inline-flex items-center gap-1">
                      {flexRender(h.column.columnDef.header, h.getContext())}
                      {sortable &&
                        (sorted === "asc" ? (
                          <ArrowUp className="size-3" />
                        ) : sorted === "desc" ? (
                          <ArrowDown className="size-3" />
                        ) : (
                          <ChevronsUpDown className="size-3 opacity-30" />
                        ))}
                    </span>
                  </th>
                );
              })}
            </tr>
          ))}
        </thead>
        <tbody>
          {table.getRowModel().rows.map((r) => (
            <tr key={r.id} className="border-b last:border-0">
              {r.getVisibleCells().map((c) => (
                <td key={c.id} className="py-2 pr-4 align-top">
                  {flexRender(c.column.columnDef.cell, c.getContext())}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
