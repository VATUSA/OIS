import {useMemo, useState} from "react";
import {
  Button,
  buttonVariants,
  Card,
  DataTable,
  type DataColumn,
  EmptyState,
  Input,
  MetricCard,
  QueryState,
  SegmentedControl,
  StatusPill,
  TimeSeries,
} from "@ois/ui";
import {Link} from "@tanstack/react-router";
import {ArrowDownToLine, ArrowUpFromLine, Clock, Film, Hash, Lock, Plane, TrendingUp, type LucideIcon} from "lucide-react";

import {usePageHeader} from "@/components/shell/page-meta";
import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {
  type CaptureSummary,
  type KeyCount,
  type NetworkPoint,
  type StatsFlightSummary,
  useAirportMovements,
  useAirportsTop,
  useAirportStats,
  useCaptures,
  useNetworkHistory,
} from "@/lib/stats";
import {toneOf} from "@/lib/status";
import {formatZulu, formatZuluFull} from "@/lib/time";

const RANGES = [
  { id: "24h", label: "24h", days: 1 },
  { id: "7d", label: "7d", days: 7 },
  { id: "30d", label: "30d", days: 30 },
] as const;
type RangeId = (typeof RANGES)[number]["id"];
const RANGE_OPTIONS = RANGES.map((r) => ({ value: r.id, label: r.label }));

const DIR_OPTIONS = [
  { value: "arr", label: "Arrivals" },
  { value: "dep", label: "Departures" },
] as const;

const normIcao = (s: string) => s.replace(/[^a-zA-Z0-9]/g, "").toUpperCase().slice(0, 4);

const NETWORK_SERIES = [
  { key: "avg", label: "Avg pilots", value: (p: NetworkPoint) => p.avg_pilots ?? 0, color: "series-1" },
  { key: "peak", label: "Peak pilots", value: (p: NetworkPoint) => p.peak_pilots ?? 0, color: "series-6" },
];
const hourOf = (p: NetworkPoint) => new Date(p.hour);

/** A section title: 20/700 with a faint leading icon, an optional description and trailing controls. */
function SectionTitle({
  icon: Icon,
  title,
  sub,
  children,
}: {
  icon: LucideIcon;
  title: string;
  sub?: string;
  children?: React.ReactNode;
}) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-2">
      <div className="flex min-w-0 flex-col gap-0.5">
        <h2 className="flex items-center gap-2 text-xl font-bold">
          <Icon className="size-4 text-ink-3" />
          {title}
        </h2>
        {sub && <p className="text-sm text-ink-2">{sub}</p>}
      </div>
      {children}
    </div>
  );
}

/** Average (area) and peak (line) pilots per hour over the window. */
function NetworkChart({ points }: { points: NetworkPoint[] }) {
  if (points.length < 2)
    return <EmptyState>Not enough data yet — collection accrues from now forward.</EmptyState>;
  return (
    <TimeSeries
      label="Network pilots over time"
      data={points}
      x={hourOf}
      series={NETWORK_SERIES}
      kind="area"
      xFormat={(v) => formatZulu(new Date(v).toISOString())}
      height={220}
    />
  );
}

/** ICAO → count rows (optionally ranked); the caller makes a row click pick that airport. */
function keyCountColumns(keyHeader: string, ranked: boolean): DataColumn<KeyCount>[] {
  const cols: DataColumn<KeyCount>[] = [
    {
      id: "icao",
      accessorFn: (r) => r.key ?? "?",
      header: keyHeader,
      icon: Plane,
      mono: true,
      cell: (c) => <span className="font-semibold text-ink">{c.getValue<string>()}</span>,
    },
    { accessorKey: "count", header: "Count", icon: Hash, mono: true, align: "right" },
  ];
  if (ranked)
    cols.unshift({
      id: "rank",
      header: "#",
      mono: true,
      align: "right",
      enableSorting: false,
      cell: (c) => <span className="text-ink-3">{c.row.index + 1}</span>,
    });
  return cols;
}
const BUSIEST_COLUMNS = keyCountColumns("Airport", true);
const DEST_COLUMNS = keyCountColumns("Destination", false);
const ORIGIN_COLUMNS = keyCountColumns("Origin", false);

function movementColumns(dir: "arr" | "dep"): DataColumn<StatsFlightSummary>[] {
  return [
    {
      accessorKey: "callsign",
      header: "Callsign",
      icon: Plane,
      mono: true,
      cell: (c) => (
        <Link
          to="/admin/historical/flights/$flightId"
          params={{ flightId: String(c.row.original.session_id) }}
          className="font-semibold text-brand-ink hover:underline"
        >
          {c.getValue<string>()}
        </Link>
      ),
    },
    {
      id: "other",
      accessorFn: (m) => (dir === "arr" ? m.departure : m.arrival) ?? "—",
      header: dir === "arr" ? "From" : "To",
      mono: true,
      cell: (c) => <span className="text-ink-2">{c.getValue<string>()}</span>,
    },
    { id: "type", accessorFn: (m) => m.aircraft_short ?? "—", header: "Type", mono: true },
    {
      accessorKey: "logon_time",
      header: "Logon",
      icon: Clock,
      mono: true,
      cell: (c) => <span className="text-ink-2">{formatZulu(c.getValue<string>())}</span>,
    },
  ];
}
const MOVEMENT_COLUMNS = { arr: movementColumns("arr"), dep: movementColumns("dep") };

function AirportLookup({ icao, onPick }: { icao: string | null; onPick: (icao: string) => void }) {
  const [entry, setEntry] = useState("");
  const stats = useAirportStats(icao);
  const [dir, setDir] = useState<"arr" | "dep">("arr");
  const movements = useAirportMovements(icao, dir);

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    const v = normIcao(entry);
    if (v.length >= 3) onPick(v);
  };

  const s = stats.data;

  return (
    <Card className="flex min-w-0 flex-col gap-4 p-5">
      <SectionTitle icon={Plane} title="Airport activity" sub="All-time recorded departures and arrivals for a US airport.">
        <form className="flex items-center gap-2" onSubmit={submit}>
          <Input
            className="h-8 w-28 font-mono uppercase"
            value={entry}
            onChange={(e) => setEntry(normIcao(e.target.value))}
            placeholder="KATL"
            aria-label="Airport ICAO"
          />
          <Button type="submit" size="sm" disabled={normIcao(entry).length < 3}>
            Look up
          </Button>
        </form>
      </SectionTitle>

      {!icao ? (
        <EmptyState icon={Plane}>Enter an ICAO, or pick one from the busiest list.</EmptyState>
      ) : (
        <QueryState
          isLoading={!s && !stats.isError}
          isError={stats.isError}
          onRetry={() => stats.refetch()}
          loading={`Loading ${icao}…`}
        >
          {s && (
            <div className="flex flex-col gap-5">
              <div className="grid gap-3 sm:grid-cols-2">
                <MetricCard label={`${icao} departures`} icon={ArrowUpFromLine} value={s.departures} />
                <MetricCard label={`${icao} arrivals`} icon={ArrowDownToLine} value={s.arrivals} />
              </div>

              {s.top_aircraft.length > 0 && (
                <div className="flex flex-col gap-2">
                  <h3 className="text-sm font-semibold">Top aircraft</h3>
                  <div className="flex flex-wrap gap-1.5">
                    {s.top_aircraft.map((a) => (
                      <StatusPill key={a.key ?? "?"} tone="neutral" className="font-mono">
                        {a.key ?? "?"}
                        <span className="text-ink-3">{a.count}</span>
                      </StatusPill>
                    ))}
                  </div>
                </div>
              )}

              <div className="grid gap-4 sm:grid-cols-2">
                <DataTable
                  label="Top destinations"
                  columns={DEST_COLUMNS}
                  data={s.top_destinations}
                  getRowId={(r, i) => r.key ?? String(i)}
                  onRowClick={(r) => r.key && onPick(r.key)}
                  empty="No data."
                />
                <DataTable
                  label="Top origins"
                  columns={ORIGIN_COLUMNS}
                  data={s.top_origins}
                  getRowId={(r, i) => r.key ?? String(i)}
                  onRowClick={(r) => r.key && onPick(r.key)}
                  empty="No data."
                />
              </div>

              <div className="flex flex-col gap-2">
                <div className="flex items-center justify-between gap-2">
                  <h3 className="text-sm font-semibold">Recent movements</h3>
                  <SegmentedControl
                    size="sm"
                    aria-label="Movement direction"
                    value={dir}
                    onChange={setDir}
                    options={DIR_OPTIONS}
                  />
                </div>
                <DataTable
                  label="Recent movements"
                  columns={MOVEMENT_COLUMNS[dir]}
                  data={movements.data ?? []}
                  getRowId={(m) => String(m.session_id)}
                  rowCap={20}
                  isLoading={movements.isLoading}
                  isError={movements.isError}
                  onRetry={() => movements.refetch()}
                  empty={`No recent ${dir === "arr" ? "arrivals" : "departures"}.`}
                />
              </div>
            </div>
          )}
        </QueryState>
      )}
    </Card>
  );
}

export function StatsPage() {
  const { data: me } = useMe();
  const canRead = hasPermission(me, "stats.data.read");
  const [range, setRange] = useState<RangeId>("7d");
  const [icao, setIcao] = useState<string | null>(null);

  usePageHeader({
    subtitle: "Historical VATSIM activity collected from the live feed (US-relevant traffic + saved event captures).",
  });

  const { from, to } = useMemo(() => {
    const days = RANGES.find((r) => r.id === range)!.days;
    const now = Date.now();
    return {
      from: new Date(now - days * 86_400_000).toISOString(),
      to: new Date(now).toISOString(),
    };
  }, [range]);

  const history = useNetworkHistory(from, to);
  const top = useAirportsTop(15);

  if (!canRead) {
    return <EmptyState icon={Lock}>You don&apos;t have access to network statistics.</EmptyState>;
  }

  return (
    <div className="flex w-full flex-col gap-6">
      <Card className="flex flex-col gap-3 p-5">
        <SectionTitle icon={TrendingUp} title="Pilots online">
          <SegmentedControl aria-label="Range" value={range} onChange={setRange} options={RANGE_OPTIONS} />
        </SectionTitle>
        <QueryState
          isLoading={!history.data && !history.isError}
          isError={history.isError}
          onRetry={() => history.refetch()}
        >
          {history.data && <NetworkChart points={history.data} />}
        </QueryState>
      </Card>

      <div className="grid gap-6 lg:grid-cols-[320px_1fr]">
        <Card className="flex min-w-0 flex-col gap-3 self-start p-5">
          <SectionTitle icon={Hash} title="Busiest airports" />
          <DataTable
            label="Busiest airports"
            columns={BUSIEST_COLUMNS}
            data={top.data ?? []}
            getRowId={(r, i) => r.key ?? String(i)}
            rowCap={15}
            onRowClick={(r) => r.key && setIcao(r.key)}
            isLoading={top.isLoading}
            isError={top.isError}
            onRetry={() => top.refetch()}
            empty="No data yet."
          />
        </Card>

        <AirportLookup icao={icao} onPick={setIcao} />
      </div>

      <SavedCaptures />
    </div>
  );
}

const CAPTURE_COLUMNS: DataColumn<CaptureSummary>[] = [
  {
    id: "capture",
    accessorFn: (c) => c.event_title || c.label || "Capture",
    header: "Capture",
    icon: Film,
    cell: (c) => <span className="font-semibold">{c.getValue<string>()}</span>,
  },
  {
    accessorKey: "start_time",
    header: "Window",
    icon: Clock,
    mono: true,
    cell: (c) => (
      <span className="whitespace-nowrap text-ink-2">
        {formatZuluFull(c.row.original.start_time)}
        {c.row.original.end_time ? ` – ${formatZulu(c.row.original.end_time)}` : " – live"}
      </span>
    ),
  },
  {
    accessorKey: "status",
    header: "Status",
    cell: (c) => {
      const open = c.getValue<string>() === "open";
      return (
        <StatusPill tone={toneOf("recording", open ? "recording" : "recorded")}>
          {open ? "recording" : "saved"}
        </StatusPill>
      );
    },
  },
  {
    id: "actions",
    header: "",
    align: "right",
    enableSorting: false,
    cell: (c) => (
      <Link
        to="/admin/historical/replay"
        search={{ capture: c.row.original.id }}
        className={buttonVariants({ variant: "secondary", size: "sm" })}
      >
        <Film className="size-3.5" />
        Replay
      </Link>
    ),
  },
];

function SavedCaptures() {
  const captures = useCaptures();
  const rows = captures.data ?? [];
  if (captures.data && rows.length === 0) return null;

  return (
    <section className="flex flex-col gap-3">
      <SectionTitle icon={Film} title="Saved captures" sub="Replay a recorded event or window on the map." />
      <DataTable
        label="Saved captures"
        columns={CAPTURE_COLUMNS}
        data={rows}
        getRowId={(c) => c.id}
        isLoading={captures.isLoading}
        isError={captures.isError}
        onRetry={() => captures.refetch()}
      />
    </section>
  );
}
