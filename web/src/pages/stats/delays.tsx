import {useEffect, useMemo, useState} from "react";
import {
  Button,
  Card,
  DataTable,
  type DataColumn,
  EmptyState,
  FilterBar,
  MetricCard,
  QueryState,
  SegmentedControl,
  Select,
  StatusPill,
  Switch,
} from "@ois/ui";
import {ArrowLeft, Clock, Hash, Lock, Plane, Timer} from "lucide-react";

import {usePageHeader} from "@/components/shell/page-meta";
import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {useDelaySummary, type DelayGroup} from "@/lib/stats";

const WINDOWS: { h: number; label: string }[] = [
  { h: 6, label: "6h" },
  { h: 24, label: "24h" },
  { h: 72, label: "3d" },
  { h: 168, label: "7d" },
  { h: 720, label: "30d" },
];
const WINDOW_OPTIONS = WINDOWS.map((w) => ({ value: String(w.h), label: w.label }));

const KIND_OPTIONS = [
  { value: "departure", label: "Departures" },
  { value: "arrival", label: "Arrivals" },
] as const;

type Level = "ok" | "watch" | "over";
const LEVEL_BG: Record<Level, string> = { ok: "bg-level-ok", watch: "bg-level-watch", over: "bg-level-over" };
const LEVEL_TEXT: Record<Level, string> = { ok: "text-level-ok", watch: "text-level-watch", over: "text-level-over" };

/** Seconds → `M:SS`. */
function fmtDur(sec: number): string {
  const m = Math.floor(sec / 60);
  const s = sec % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

/**
 * Level a median. Normalized: relative to the airport baseline (≤1.1× ok, ≤1.4× watch, else over).
 * Fixed: ok ≤20 min, watch ≤40 min, over beyond.
 */
function levelFor(median: number, baseline: number | null, normalize: boolean): Level {
  if (normalize && baseline && baseline > 0) {
    const r = median / baseline;
    return r <= 1.1 ? "ok" : r <= 1.4 ? "watch" : "over";
  }
  return median <= 1200 ? "ok" : median <= 2400 ? "watch" : "over";
}

/** Ranked groups: key (clickable to drill when `onPick`), a median meter, median, n and p90. */
function groupColumns({
  keyHeader,
  groups,
  baseline,
  normalize,
  onPick,
}: {
  keyHeader: string;
  groups: DelayGroup[];
  baseline: number | null;
  normalize: boolean;
  onPick?: (key: string) => void;
}): DataColumn<DelayGroup>[] {
  const max = Math.max(1, ...groups.map((g) => g.median_sec));
  return [
    {
      accessorKey: "key",
      header: keyHeader,
      icon: Plane,
      mono: true,
      cell: (c) =>
        onPick ? (
          <button
            type="button"
            onClick={() => onPick(c.getValue<string>())}
            className="font-semibold text-brand-ink hover:underline"
          >
            {c.getValue<string>()}
          </button>
        ) : (
          <span className="font-semibold">{c.getValue<string>()}</span>
        ),
    },
    {
      id: "meter",
      header: "",
      enableSorting: false,
      headerClassName: "w-full",
      cell: (c) => {
        const g = c.row.original;
        return (
          <div className="h-1.5 min-w-24 rounded-full bg-line-soft">
            <div
              className={`h-full rounded-full ${LEVEL_BG[levelFor(g.median_sec, baseline, normalize)]}`}
              style={{ width: `${(g.median_sec / max) * 100}%`, minWidth: 4 }}
            />
          </div>
        );
      },
    },
    {
      accessorKey: "median_sec",
      header: "Median",
      icon: Timer,
      mono: true,
      align: "right",
      cell: (c) => {
        const g = c.row.original;
        return (
          <span className={`font-semibold ${LEVEL_TEXT[levelFor(g.median_sec, baseline, normalize)]}`}>
            {fmtDur(g.median_sec)}
          </span>
        );
      },
    },
    { accessorKey: "count", header: "n", icon: Hash, mono: true, align: "right" },
    {
      accessorKey: "p90_sec",
      header: "p90",
      icon: Clock,
      mono: true,
      align: "right",
      cell: (c) => <span className="text-ink-2">{fmtDur(c.getValue<number>())}</span>,
    },
  ];
}

export function DelaysPage() {
  const { data: me } = useMe();
  const canRead = hasPermission(me, "stats.data.read");

  const [kind, setKind] = useState<"departure" | "arrival">("departure");
  const [airport, setAirport] = useState("");
  const [runway, setRunway] = useState("");
  const [procedure, setProcedure] = useState("");
  const [hours, setHours] = useState(24);
  const [normalize, setNormalize] = useState(false);
  const [page, setPage] = useState(1);

  usePageHeader({
    subtitle:
      "Taxi-out and arrival transit times from live radar. Color by fixed thresholds, or normalized to each airport's baseline.",
  });

  // A filter change invalidates whatever page was showing (the by-airport list underneath it
  // shifts) — start back at the first page.
  useEffect(() => {
    setPage(1);
  }, [kind, airport, runway, procedure, hours]);

  const summary = useDelaySummary({
    kind,
    airport: airport || undefined,
    runway: runway || undefined,
    procedure: procedure || undefined,
    hours,
    page,
    pageSize: 25,
  });

  const d = summary.data;
  const baseline = d?.overall.median_sec ?? null; // the airport's overall median (when filtered)
  const metric = kind === "departure" ? "taxi-out" : "arrival transit";
  const procLabel = kind === "departure" ? "SID" : "STAR";

  const pickAirport = (a: string) => {
    setAirport(a);
    setRunway("");
    setProcedure("");
  };

  // Column sets close over each list's max (for the meter) — rebuilt only when their data changes.
  const airportColumns = useMemo(
    () => groupColumns({ keyHeader: "Airport", groups: d?.by_airport ?? [], baseline: null, normalize: false, onPick: pickAirport }),
    // pickAirport only calls stable setters, so the groups are the only dependency.
    [d?.by_airport],
  );
  const runwayColumns = useMemo(
    () => groupColumns({ keyHeader: "Runway", groups: d?.by_runway ?? [], baseline, normalize }),
    [d?.by_runway, baseline, normalize],
  );
  const procedureColumns = useMemo(
    () => groupColumns({ keyHeader: procLabel, groups: d?.by_procedure ?? [], baseline, normalize }),
    [d?.by_procedure, baseline, normalize, procLabel],
  );

  if (!canRead) {
    return <EmptyState icon={Lock}>You don&apos;t have access to network statistics.</EmptyState>;
  }

  return (
    <div className="flex flex-col gap-6">
      <FilterBar>
        <SegmentedControl aria-label="Direction" value={kind} onChange={setKind} options={KIND_OPTIONS} />
        <SegmentedControl
          aria-label="Window"
          value={String(hours)}
          onChange={(v) => setHours(Number(v))}
          options={WINDOW_OPTIONS}
        />

        {airport ? (
          <>
            <Button size="sm" variant="ghost" onClick={() => pickAirport("")}>
              <ArrowLeft className="size-4" /> All airports
            </Button>
            <StatusPill tone="brand" className="font-mono">
              {airport}
            </StatusPill>
            {d && d.by_runway.length > 0 && (
              <Select size="sm" aria-label="Runway" value={runway} onChange={(e) => setRunway(e.target.value)}>
                <option value="">All runways</option>
                {d.by_runway.map((g) => (
                  <option key={g.key} value={g.key}>
                    Rwy {g.key}
                  </option>
                ))}
              </Select>
            )}
            {d && d.by_procedure.length > 0 && (
              <Select size="sm" aria-label={procLabel} value={procedure} onChange={(e) => setProcedure(e.target.value)}>
                <option value="">All {procLabel}s</option>
                {d.by_procedure.map((g) => (
                  <option key={g.key} value={g.key}>
                    {g.key}
                  </option>
                ))}
              </Select>
            )}
          </>
        ) : (
          <span className="text-xs text-ink-3">Pick an airport below to drill in.</span>
        )}

        <label className="ml-auto flex items-center gap-2 text-xs text-ink-2">
          <Switch checked={normalize} onCheckedChange={setNormalize} />
          Normalize per airport
        </label>
      </FilterBar>

      <QueryState isLoading={!d && !summary.isError} isError={!d && summary.isError} onRetry={() => summary.refetch()}>
        {d && (
          <>
            <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
              <MetricCard label={`Median ${metric}`} icon={Timer} value={fmtDur(d.overall.median_sec)} />
              <MetricCard label="Average" icon={Clock} value={fmtDur(d.overall.avg_sec)} />
              <MetricCard label="p90" icon={Clock} value={fmtDur(d.overall.p90_sec)} />
              <MetricCard label="Legs" icon={Hash} value={d.overall.count} />
            </div>

            {d.overall.count === 0 ? (
              <EmptyState>No {metric} data in this window yet — the collector builds it from live traffic.</EmptyState>
            ) : !airport ? (
              <section className="flex flex-col gap-3">
                <div className="flex items-baseline justify-between gap-2">
                  <h2 className="text-xl font-bold">By airport · median {metric}</h2>
                  <span className="font-mono text-xs text-ink-3">{d.by_airport_total} airports</span>
                </div>
                <DataTable
                  label={`By airport, median ${metric}`}
                  columns={airportColumns}
                  data={d.by_airport}
                  getRowId={(g) => g.key}
                  rowCap={25}
                  serverPagination={{
                    page: d.page,
                    pageSize: d.page_size,
                    total: d.by_airport_total,
                    onPageChange: setPage,
                  }}
                />
              </section>
            ) : (
              <div className="grid gap-4 lg:grid-cols-2">
                <Card className="flex min-w-0 flex-col gap-3 p-5">
                  <div className="flex items-center justify-between gap-2">
                    <h2 className="text-xl font-bold">By runway</h2>
                    {normalize && (
                      <StatusPill tone="neutral" className="font-mono">
                        vs {fmtDur(baseline ?? 0)}
                      </StatusPill>
                    )}
                  </div>
                  <DataTable
                    label="By runway"
                    columns={runwayColumns}
                    data={d.by_runway}
                    getRowId={(g) => g.key}
                    rowCap={25}
                    empty="No runway data."
                  />
                </Card>
                <Card className="flex min-w-0 flex-col gap-3 p-5">
                  <h2 className="text-xl font-bold">By {procLabel}</h2>
                  <DataTable
                    label={`By ${procLabel}`}
                    columns={procedureColumns}
                    data={d.by_procedure}
                    getRowId={(g) => g.key}
                    rowCap={25}
                    empty="No procedure data."
                  />
                </Card>
              </div>
            )}
          </>
        )}
      </QueryState>
    </div>
  );
}
