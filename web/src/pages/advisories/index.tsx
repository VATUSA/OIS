import {useMemo} from "react";
import {Link} from "@tanstack/react-router";
import {buttonVariants, type DataColumn, DataTable, MetricCard, QueryState, StatusPill} from "@ois/ui";
import {Gauge, OctagonX, RefreshCw, Split, Timer, Waypoints} from "lucide-react";

import {usePageHeader} from "@/components/shell/page-meta";
import {
  type PublicGdp,
  type PublicGroundStop,
  type PublicProgram,
  type PublicRestriction,
  usePublicBoard,
} from "@/lib/public";

/** "1423z" from an ISO datetime (Zulu). */
function zulu(iso: string): string {
  const d = new Date(iso);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getUTCHours())}${p(d.getUTCMinutes())}z`;
}

/** "1400z" from a bare HHMM clock string. */
function hhmm(s: string): string {
  return /^\d{4}$/.test(s) ? `${s}z` : s;
}

function scopeLabel(scope: string): string {
  const codes = scope.trim();
  return codes === "" ? "field-wide" : codes;
}

/** Live inbound demand (next 60 min), flagged when it exceeds the AAR. */
function Demand({ demand, over }: { demand: number; over: boolean }) {
  return (
    <span className="inline-flex items-center justify-end gap-2">
      {over && <StatusPill tone="bad">over</StatusPill>}
      <span className={over ? "text-danger" : undefined}>{demand}/hr</span>
    </span>
  );
}

function spacing(trail: number, mit: number): string {
  if (mit > 0) return `${mit} MIT`;
  if (trail > 0) return `${trail} MINIT`;
  return "AAR only";
}

const airportCell = (v: string) => <span className="font-semibold text-ink">{v}</span>;

const groundStopColumns: DataColumn<PublicGroundStop>[] = [
  { accessorKey: "airport", header: "Airport", mono: true, cell: (c) => airportCell(c.row.original.airport) },
  { id: "scope", accessorFn: (r) => scopeLabel(r.scope), header: "Scope", mono: true },
  {
    id: "until",
    accessorFn: (r) => r.until ?? "",
    header: "Until",
    mono: true,
    align: "right",
    cell: (c) => (c.row.original.until ? hhmm(c.row.original.until) : "further notice"),
  },
];

const gdpColumns: DataColumn<PublicGdp>[] = [
  { accessorKey: "airport", header: "Airport", mono: true, cell: (c) => airportCell(c.row.original.airport) },
  { accessorKey: "aar", header: "AAR", mono: true, align: "right" },
  {
    accessorKey: "demand_60min",
    header: "Demand",
    mono: true,
    align: "right",
    cell: (c) => <Demand demand={c.row.original.demand_60min} over={c.row.original.over_capacity} />,
  },
  { id: "scope", accessorFn: (r) => scopeLabel(r.scope), header: "Scope", mono: true },
  {
    id: "enroute",
    accessorFn: (r) => r.max_enroute_min ?? -1,
    header: "Distance",
    mono: true,
    align: "right",
    cell: (c) => (c.row.original.max_enroute_min != null ? `≤${c.row.original.max_enroute_min}min out` : "—"),
  },
  {
    id: "window",
    accessorFn: (r) => r.start_time,
    header: "Window",
    mono: true,
    cell: (c) => `${hhmm(c.row.original.start_time)}–${hhmm(c.row.original.end_time)}`,
  },
  {
    accessorKey: "controlled",
    header: "Delayed",
    mono: true,
    align: "right",
    cell: (c) =>
      c.row.original.controlled > 0 ? c.row.original.controlled : <span className="text-ink-3">none</span>,
  },
  {
    accessorKey: "avg_delay_min",
    header: "Avg",
    mono: true,
    align: "right",
    cell: (c) => (c.row.original.controlled > 0 ? `${c.row.original.avg_delay_min}′` : "—"),
  },
  {
    accessorKey: "max_delay_min",
    header: "Max",
    mono: true,
    align: "right",
    cell: (c) => (c.row.original.controlled > 0 ? `${c.row.original.max_delay_min}′` : "—"),
  },
];

const restrictionColumns: DataColumn<PublicRestriction>[] = [
  {
    id: "restriction",
    accessorFn: (r) => r.decoded || r.restriction,
    header: "Restriction",
    cell: (c) => {
      const r = c.row.original;
      return (
        <div className="flex flex-col gap-0.5">
          <span className="font-semibold text-ink">{r.decoded || r.restriction}</span>
          {r.decoded && <span className="font-mono text-xs text-ink-3">{r.restriction}</span>}
        </div>
      );
    },
  },
  { accessorKey: "requesting", header: "Requesting", mono: true },
  { accessorKey: "providing", header: "Providing", mono: true },
  {
    id: "window",
    accessorFn: (r) => r.start_time,
    header: "Window",
    mono: true,
    align: "right",
    cell: (c) => {
      const r = c.row.original;
      return `${zulu(r.start_time)}${r.stop_time ? `–${zulu(r.stop_time)}` : " · UFN"}`;
    },
  },
];

const programColumns: DataColumn<PublicProgram>[] = [
  { accessorKey: "icao", header: "Airport", mono: true, cell: (c) => airportCell(c.row.original.icao) },
  { accessorKey: "aar", header: "AAR", mono: true, align: "right" },
  {
    accessorKey: "demand_60min",
    header: "Demand",
    mono: true,
    align: "right",
    cell: (c) => <Demand demand={c.row.original.demand_60min} over={c.row.original.over_capacity} />,
  },
  { id: "spacing", accessorFn: (r) => spacing(r.trail, r.mit), header: "Spacing", mono: true },
  {
    accessorKey: "jets_only",
    header: "Types",
    cell: (c) =>
      c.row.original.jets_only ? <StatusPill tone="neutral">jets only</StatusPill> : <span className="text-ink-3">all</span>,
  },
  {
    id: "gates",
    accessorFn: (r) => r.gates.length,
    header: "Gates",
    mono: true,
    align: "right",
    cell: (c) => c.row.original.gates.length || "—",
  },
];

/** One initiative type: a section title with its live count, then its table. */
function Section<T extends object>({
  title,
  icon: Icon,
  alert,
  columns,
  rows,
  getRowId,
  empty,
}: {
  title: string;
  icon: typeof Gauge;
  alert?: boolean;
  columns: DataColumn<T>[];
  rows: readonly T[];
  getRowId: (row: T) => string;
  empty: string;
}) {
  return (
    <section className="flex flex-col gap-3">
      <div className="flex items-center gap-2">
        <Icon className={`size-4 ${alert ? "text-danger" : "text-ink-3"}`} />
        <h2 className="text-xl font-bold">{title}</h2>
        <span className="font-mono text-sm text-ink-3">{rows.length}</span>
      </div>
      <DataTable columns={columns} data={rows} getRowId={getRowId} rowCap={25} empty={empty} label={title} />
    </section>
  );
}

export function AdvisoriesPage() {
  const board = usePublicBoard();
  const b = board.data;
  const asOf = b?.as_of;
  const fetching = board.isFetching;

  const actions = useMemo(
    () => (
      <div className="flex items-center gap-3">
        {asOf && (
          <span className="flex items-center gap-1.5 font-mono text-xs text-ink-3">
            <RefreshCw className={`size-3 ${fetching ? "animate-spin" : ""}`} />
            updated {zulu(asOf)}
          </span>
        )}
        <Link to="/advisories/fcas" className={buttonVariants({ variant: "outline", size: "sm" })}>
          <Waypoints />
          FCA overview
        </Link>
      </div>
    ),
    [asOf, fetching],
  );
  const total = b ? b.ground_stops.length + b.gdps.length + b.restrictions.length + b.programs.length : null;
  usePageHeader({
    subtitle: "Active traffic management initiatives across the NAS.",
    count: total,
    actions,
  });

  return (
    <QueryState
      isLoading={board.isLoading}
      isError={board.isError}
      loading="Loading advisories…"
      error="Couldn’t load advisories. Retrying…"
      onRetry={() => board.refetch()}
    >
      {b && (
        <div className="flex flex-col gap-6">
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
            <MetricCard
              label="Ground stops"
              icon={OctagonX}
              value={b.ground_stops.length}
              tone={b.ground_stops.length > 0 ? "bad" : undefined}
            />
            <MetricCard label="Ground delay programs" icon={Timer} value={b.gdps.length} />
            <MetricCard label="Restrictions" icon={Split} value={b.restrictions.length} />
            <MetricCard label="Rate programs" icon={Gauge} value={b.programs.length} />
          </div>

          <Section
            title="Ground Stops"
            icon={OctagonX}
            alert={b.ground_stops.length > 0}
            columns={groundStopColumns}
            rows={b.ground_stops}
            getRowId={(r) => r.id}
            empty="No active ground stops."
          />
          <Section
            title="Ground Delay Programs"
            icon={Timer}
            columns={gdpColumns}
            rows={b.gdps}
            getRowId={(r) => r.id}
            empty="No active ground delay programs."
          />
          <Section
            title="Restrictions"
            icon={Split}
            columns={restrictionColumns}
            rows={b.restrictions}
            getRowId={(r) => r.id}
            empty="No active restrictions."
          />
          <Section
            title="Rate Programs"
            icon={Gauge}
            columns={programColumns}
            rows={b.programs}
            getRowId={(r) => r.icao}
            empty="No active rate programs."
          />
        </div>
      )}
    </QueryState>
  );
}
