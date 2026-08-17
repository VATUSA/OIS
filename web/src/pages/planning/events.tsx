import {useMemo, useState} from "react";
import {Badge, Button, Card, CardContent} from "@ois/ui";
import {useNavigate} from "@tanstack/react-router";
import {
  type ColumnDef,
  flexRender,
  getCoreRowModel,
  getSortedRowModel,
  type SortingState,
  useReactTable,
} from "@tanstack/react-table";
import {ArrowDown, ArrowUp, ChevronsUpDown, ExternalLink, SlidersHorizontal} from "lucide-react";

import {useMe} from "@/lib/auth";
import {type EventSummary, useUpcomingEvents, vatusaEditUrl} from "@/lib/events";
import {hasPermission} from "@/lib/permissions";
import {formatZuluFull} from "@/lib/time";

function reviewVariant(status: string): "success" | "secondary" | "outline" {
  if (status === "approved") return "success";
  if (status === "rejected" || status === "denied") return "outline";
  return "secondary";
}

type Scope = "upcoming" | "past" | "all";
const EMPTY_SORTING: SortingState = [];

function endMs(e: EventSummary): number {
  const t = new Date(e.end_time).getTime();
  return Number.isNaN(t) ? 0 : t;
}

function EventActions({ event }: { event: EventSummary }) {
  const navigate = useNavigate();
  const editUrl = vatusaEditUrl(event);
  return (
    <div className="flex items-center justify-end gap-1">
      {editUrl && (
        <a
          href={editUrl}
          target="_blank"
          rel="noreferrer"
          onClick={(e) => e.stopPropagation()}
          title="Edit on VATUSA"
          className="rounded-md p-1.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        >
          <ExternalLink className="size-4" />
        </a>
      )}
      <Button
        size="sm"
        onClick={(e) => {
          e.stopPropagation();
          navigate({ to: "/planning/events/$eventId", params: { eventId: String(event.id) } });
        }}
      >
        <SlidersHorizontal className="size-3.5" />
        Manage
      </Button>
    </div>
  );
}

function useEventColumns(): ColumnDef<EventSummary>[] {
  return useMemo(
    () => [
      { accessorKey: "title", header: "Name", cell: (c) => <span className="font-medium">{String(c.getValue())}</span> },
      {
        accessorKey: "facility",
        header: "Facility",
        cell: (c) => <span className="font-mono text-xs">{String(c.getValue() || "—")}</span>,
      },
      {
        accessorKey: "start_time",
        header: "Start (Z)",
        cell: (c) => <span className="font-mono text-xs">{formatZuluFull(String(c.getValue()))}</span>,
      },
      {
        accessorKey: "end_time",
        header: "End (Z)",
        cell: (c) => (
          <span className="font-mono text-xs text-muted-foreground">
            {formatZuluFull(String(c.getValue()))}
          </span>
        ),
      },
      {
        accessorKey: "review_status",
        header: "Status",
        cell: (c) => {
          const s = String(c.getValue() ?? "");
          return s ? <Badge variant={reviewVariant(s)}>{s}</Badge> : <span className="text-muted-foreground">—</span>;
        },
      },
      {
        id: "actions",
        header: () => <span className="sr-only">Actions</span>,
        enableSorting: false,
        cell: (c) => <EventActions event={c.row.original} />,
      },
    ],
    [],
  );
}

function EventsTable({ events }: { events: EventSummary[] }) {
  const navigate = useNavigate();
  const columns = useEventColumns();
  const [sorting, setSorting] = useState<SortingState>([{ id: "start_time", desc: false }]);
  const table = useReactTable({
    data: events,
    columns,
    state: { sorting },
    onSortingChange: setSorting,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
    autoResetPageIndex: false,
    autoResetExpanded: false,
  });

  return (
    <div className="overflow-x-auto">
      <table className="w-full text-sm">
        <thead>
          {table.getHeaderGroups().map((hg) => (
            <tr key={hg.id} className="text-left text-xs uppercase tracking-wide text-muted-foreground">
              {hg.headers.map((h) => {
                const sortable = h.column.getCanSort();
                const sorted = h.column.getIsSorted();
                return (
                  <th
                    key={h.id}
                    className={
                      "pb-2 pr-3 font-medium " + (h.column.id === "actions" ? "text-right" : "") +
                      (sortable ? " cursor-pointer select-none" : "")
                    }
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
            <tr
              key={r.id}
              className="cursor-pointer border-t transition-colors hover:bg-accent/50"
              onClick={() =>
                navigate({ to: "/planning/events/$eventId", params: { eventId: String(r.original.id) } })
              }
            >
              {r.getVisibleCells().map((cell) => (
                <td key={cell.id} className="py-2 pr-3">
                  {flexRender(cell.column.columnDef.cell, cell.getContext())}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function PlanningEventsPage() {
  const { data: me } = useMe();
  const canPlan = hasPermission(me, "events.plan.read");
  const events = useUpcomingEvents();
  const [scope, setScope] = useState<Scope>("upcoming");

  const now = Date.now();
  const filtered = useMemo(() => {
    if (!events.data) return undefined;
    if (scope === "all") return events.data;
    return events.data.filter((e) =>
      scope === "past" ? endMs(e) < now : endMs(e) >= now,
    );
    // `now` intentionally captured once per render — the split is coarse (event granularity).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [events.data, scope]);

  const SCOPES: { id: Scope; label: string }[] = [
    { id: "upcoming", label: "Upcoming" },
    { id: "past", label: "Past" },
    { id: "all", label: "All" },
  ];

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Events</h1>
          <p className="text-muted-foreground">
            VATUSA events. Open one to plan its TMI package, DCC support, facility staffing, and
            airport rates.
          </p>
        </div>
        {canPlan && (
          <div className="flex rounded-md border p-0.5">
            {SCOPES.map((s) => (
              <button
                key={s.id}
                type="button"
                onClick={() => setScope(s.id)}
                className={
                  "rounded px-3 py-1 text-sm transition-colors " +
                  (scope === s.id
                    ? "bg-accent text-foreground"
                    : "text-muted-foreground hover:text-foreground")
                }
              >
                {s.label}
              </button>
            ))}
          </div>
        )}
      </div>

      <Card>
        <CardContent className="pt-6">
          {!canPlan ? (
            <p className="py-8 text-center text-sm text-muted-foreground">
              You don&apos;t have event planning access yet.
            </p>
          ) : events.isError ? (
            <p className="py-8 text-center text-sm text-muted-foreground">Couldn&apos;t load events.</p>
          ) : !filtered ? (
            <p className="py-8 text-center text-sm text-muted-foreground">Loading…</p>
          ) : filtered.length === 0 ? (
            <p className="py-8 text-center text-sm text-muted-foreground">
              {scope === "past"
                ? "No recent past events."
                : "No upcoming events on the VATUSA calendar right now."}
            </p>
          ) : (
            <EventsTable events={filtered} />
          )}
        </CardContent>
      </Card>
    </div>
  );
}
