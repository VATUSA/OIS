import {useEffect, useState} from "react";
import {
  Button,
  DataTable,
  type DataColumn,
  EmptyState,
  FilterBar,
  Input,
  SegmentedControl,
  Select,
  StatusPill,
  Switch,
} from "@ois/ui";
import {Clock, DoorOpen, Lock, Plane, PlaneTakeoff, Timer} from "lucide-react";

import {ZuluDateTime} from "@/components/zulu-datetime";
import {usePageHeader} from "@/components/shell/page-meta";
import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {
  useTaxiEstimates,
  useTaxiObservations,
  type TaxiInsightsFilters,
} from "@/lib/taxi-insights";
import {formatZuluFull} from "@/lib/time";

const PAGE_SIZE = 50;

const normIcao = (s: string) => s.replace(/[^a-zA-Z0-9]/g, "").toUpperCase().slice(0, 4);

const TIERS = [
  { value: "", label: "Any tier" },
  { value: "gate_type_runway", label: "Gate + type + runway" },
  { value: "airport_runway", label: "Airport + runway" },
  { value: "airport", label: "Airport-wide" },
  // No duration in this label: the default is a different flat value per metric (pushback,
  // start-up, taxi), and the actual value is already shown alongside the badge — a single
  // hardcoded duration here would be wrong for the others.
  { value: "default", label: "Default" },
];

const TAB_OPTIONS = [
  { value: "observations", label: "Observations" },
  { value: "estimates", label: "Estimates" },
] as const;

/** Seconds → "Mm Ss", or "—" when unknown. Rounds to the nearest whole second first so a
 * fractional remainder (e.g. 299.5) can't round up to "60s" instead of carrying into the minute. */
export function fmtDur(sec: number | null | undefined): string {
  if (sec == null) return "—";
  const total = Math.round(sec);
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}m ${s}s`;
}

function tierLabel(tier: string): string {
  return TIERS.find((t) => t.value === tier)?.label ?? tier;
}

type Observation = NonNullable<ReturnType<typeof useTaxiObservations>["data"]>["items"][number];
type Estimate = NonNullable<ReturnType<typeof useTaxiEstimates>["data"]>["items"][number];

const dash = (v: string | null | undefined) => v ?? "—";

const OBSERVATION_COLUMNS: DataColumn<Observation>[] = [
  {
    accessorKey: "observed_at",
    header: "Time",
    icon: Clock,
    mono: true,
    cell: (c) => <span className="whitespace-nowrap text-ink-2">{formatZuluFull(c.getValue<string>())}</span>,
  },
  {
    accessorKey: "airport",
    header: "Airport",
    icon: Plane,
    mono: true,
    cell: (c) => <span className="font-semibold">{c.getValue<string>()}</span>,
  },
  { id: "gate", accessorFn: (o) => dash(o.gate_id), header: "Gate", icon: DoorOpen, mono: true },
  { id: "aircraft", accessorFn: (o) => dash(o.aircraft), header: "Aircraft", mono: true },
  { id: "runway", accessorFn: (o) => dash(o.runway), header: "Runway", icon: PlaneTakeoff, mono: true },
  {
    accessorKey: "pushback_sec",
    header: "Pushback",
    icon: Timer,
    mono: true,
    align: "right",
    cell: (c) => fmtDur(c.getValue<number | null>()),
  },
  {
    accessorKey: "startup_sec",
    header: "Start-up",
    mono: true,
    align: "right",
    cell: (c) => fmtDur(c.getValue<number | null>()),
  },
  {
    accessorKey: "taxi_sec",
    header: "Taxi",
    mono: true,
    align: "right",
    cell: (c) => fmtDur(c.getValue<number | null>()),
  },
  {
    accessorKey: "is_outlier",
    header: "Outlier",
    cell: (c) => (c.getValue<boolean>() ? <StatusPill tone="bad">outlier</StatusPill> : null),
  },
];

/** A learned estimate: duration, the tier it fell back to, and its sample count. */
function EstimateCell({ sec, tier, n }: { sec: number | null | undefined; tier: string; n: number }) {
  return (
    <div className="flex items-center gap-1.5 whitespace-nowrap">
      <span className="font-mono">{fmtDur(sec)}</span>
      <StatusPill tone="neutral">{tierLabel(tier)}</StatusPill>
      <span className="font-mono text-xs text-ink-3">n={n}</span>
    </div>
  );
}

const ESTIMATE_COLUMNS: DataColumn<Estimate>[] = [
  {
    accessorKey: "airport",
    header: "Airport",
    icon: Plane,
    mono: true,
    cell: (c) => <span className="font-semibold">{c.getValue<string>()}</span>,
  },
  { id: "gate", accessorFn: (e) => dash(e.gate_id), header: "Gate", icon: DoorOpen, mono: true },
  { id: "aircraft", accessorFn: (e) => dash(e.aircraft), header: "Aircraft", mono: true },
  { id: "runway", accessorFn: (e) => dash(e.runway), header: "Runway", icon: PlaneTakeoff, mono: true },
  {
    accessorKey: "pushback_sec",
    header: "Pushback",
    icon: Timer,
    cell: (c) => {
      const e = c.row.original;
      return <EstimateCell sec={e.pushback_sec} tier={e.pushback_tier} n={e.pushback_sample_count} />;
    },
  },
  {
    accessorKey: "startup_sec",
    header: "Start-up",
    cell: (c) => {
      const e = c.row.original;
      return <EstimateCell sec={e.startup_sec} tier={e.startup_tier} n={e.startup_sample_count} />;
    },
  },
  {
    accessorKey: "taxi_sec",
    header: "Taxi",
    cell: (c) => {
      const e = c.row.original;
      return <EstimateCell sec={e.taxi_sec} tier={e.taxi_tier} n={e.taxi_sample_count} />;
    },
  },
];

export function TaxiInsightsPage() {
  const { data: me } = useMe();
  const canRead = hasPermission(me, "stats.data.read");

  const [tab, setTab] = useState<"observations" | "estimates">("observations");
  const [page, setPage] = useState(1);

  const [airportDraft, setAirportDraft] = useState("");
  const [gateDraft, setGateDraft] = useState("");
  const [aircraftDraft, setAircraftDraft] = useState("");
  const [runwayDraft, setRunwayDraft] = useState("");
  const [fromDraft, setFromDraft] = useState<number | null>(null);
  const [toDraft, setToDraft] = useState<number | null>(null);
  const [includeOutliers, setIncludeOutliers] = useState(true);
  const [fallbackTier, setFallbackTier] = useState("");

  const [filters, setFilters] = useState<TaxiInsightsFilters>({});

  usePageHeader({
    subtitle: "Raw departure timing observations and their learned per-gate/type/runway estimates.",
  });

  useEffect(() => {
    setPage(1);
  }, [tab, filters, fallbackTier]);

  const apply = () =>
    setFilters({
      airport: normIcao(airportDraft) || undefined,
      gateId: gateDraft.trim() || undefined,
      aircraft: aircraftDraft.trim().toUpperCase() || undefined,
      runway: runwayDraft.trim().toUpperCase() || undefined,
      from: fromDraft != null ? new Date(fromDraft * 1000).toISOString() : undefined,
      to: toDraft != null ? new Date(toDraft * 1000).toISOString() : undefined,
      includeOutliers,
    });
  const clear = () => {
    setAirportDraft("");
    setGateDraft("");
    setAircraftDraft("");
    setRunwayDraft("");
    setFromDraft(null);
    setToDraft(null);
    setIncludeOutliers(true);
    setFallbackTier("");
    setFilters({});
  };
  const hasFilters = Boolean(
    filters.airport ||
      filters.gateId ||
      filters.aircraft ||
      filters.runway ||
      filters.from ||
      filters.to ||
      !filters.includeOutliers,
  );

  const observations = useTaxiObservations(page, PAGE_SIZE, filters);
  const estimates = useTaxiEstimates(page, PAGE_SIZE, { ...filters, fallbackTier });

  if (!canRead) {
    return <EmptyState icon={Lock}>You don&apos;t have access to network statistics.</EmptyState>;
  }

  return (
    <div className="flex flex-col gap-4">
      <form
        onSubmit={(e) => {
          e.preventDefault();
          apply();
        }}
      >
        <FilterBar>
          <SegmentedControl aria-label="View" value={tab} onChange={setTab} options={TAB_OPTIONS} />
          <Input
            aria-label="Airport"
            placeholder={tab === "estimates" ? "Airport (required)" : "Airport"}
            value={airportDraft}
            onChange={(e) => setAirportDraft(e.target.value)}
            className={`h-8 font-mono uppercase placeholder:normal-case ${tab === "estimates" ? "w-40" : "w-24"}`}
          />
          <Input
            aria-label="Gate"
            placeholder="Gate id"
            value={gateDraft}
            onChange={(e) => setGateDraft(e.target.value)}
            className="h-8 w-24 font-mono"
          />
          <Input
            aria-label="Aircraft"
            placeholder="Aircraft"
            value={aircraftDraft}
            onChange={(e) => setAircraftDraft(e.target.value)}
            className="h-8 w-24 font-mono uppercase placeholder:normal-case"
          />
          <Input
            aria-label="Runway"
            placeholder="Runway"
            value={runwayDraft}
            onChange={(e) => setRunwayDraft(e.target.value)}
            className="h-8 w-20 font-mono uppercase placeholder:normal-case"
          />
          <div className="flex items-center gap-1.5 text-xs text-ink-2">
            From
            <ZuluDateTime label="From" value={fromDraft} onChange={setFromDraft} onClear={() => setFromDraft(null)} />
          </div>
          <div className="flex items-center gap-1.5 text-xs text-ink-2">
            To
            <ZuluDateTime label="To" value={toDraft} onChange={setToDraft} onClear={() => setToDraft(null)} />
          </div>
          {tab === "estimates" && (
            <Select
              size="sm"
              aria-label="Fallback tier"
              value={fallbackTier}
              onChange={(e) => setFallbackTier(e.target.value)}
            >
              {TIERS.map((t) => (
                <option key={t.value} value={t.value}>
                  {t.label}
                </option>
              ))}
            </Select>
          )}
          <label className="flex items-center gap-2 text-xs text-ink-2">
            <Switch checked={includeOutliers} onCheckedChange={setIncludeOutliers} />
            Include outliers
          </label>
          <Button type="submit" size="sm">
            Apply
          </Button>
          {hasFilters && (
            <Button type="button" size="sm" variant="ghost" onClick={clear}>
              Clear
            </Button>
          )}
        </FilterBar>
      </form>

      {tab === "observations" ? (
        <DataTable
          label="Taxi observations"
          columns={OBSERVATION_COLUMNS}
          data={observations.data?.items ?? []}
          getRowId={(o) => String(o.id)}
          rowCap={PAGE_SIZE}
          serverPagination={
            observations.data && {
              page: observations.data.page,
              pageSize: observations.data.page_size,
              total: observations.data.total,
              onPageChange: setPage,
            }
          }
          isLoading={!observations.data && !observations.isError}
          isError={!observations.data && observations.isError}
          onRetry={() => observations.refetch()}
          empty="No observations match these filters."
        />
      ) : !filters.airport ? (
        <EmptyState icon={Plane}>Enter an airport above — estimates are computed per airport.</EmptyState>
      ) : (
        <DataTable
          label="Taxi estimates"
          columns={ESTIMATE_COLUMNS}
          data={estimates.data?.items ?? []}
          rowCap={PAGE_SIZE}
          serverPagination={
            estimates.data && {
              page: estimates.data.page,
              pageSize: estimates.data.page_size,
              total: estimates.data.total,
              onPageChange: setPage,
            }
          }
          isLoading={!estimates.data && !estimates.isError}
          isError={!estimates.data && estimates.isError}
          onRetry={() => estimates.refetch()}
          empty="No estimates match these filters."
        />
      )}
    </div>
  );
}
