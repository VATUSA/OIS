import {useMemo, useState} from "react";
import {useSearch} from "@tanstack/react-router";
import {
  Bars,
  Button,
  Card,
  type DataColumn,
  DataTable,
  Donut,
  EmptyState,
  Input,
  MetricCard,
  QueryState,
  StatusPill,
  Tabs,
} from "@ois/ui";
import {Clock, Filter, Lock, Plane, PlaneLanding, PlaneTakeoff} from "lucide-react";

import {usePageHeader} from "@/components/shell/page-meta";
import type {LadderFilters} from "@/features/dashboard/types";
import {type Flow, type FlowFlight, useAirportFlow} from "@/lib/feed";
import {toneOf} from "@/lib/status";
import {hhmmZulu} from "@/lib/time";
import {DeparturesView} from "@/pages/departures";
import {TaxiView} from "@/pages/taxi";
import {ArrivalLadder} from "@/components/ladder/ArrivalLadder";

type Sub = "summary" | "aircraft" | "ladder" | "demand" | "departures" | "taxi";

const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

/** A flight state's CSS colour (unknown states read as arrived). */
function flightColor(status: string): string {
  const tone = toneOf("flight", status);
  return `var(--flight-${tone === "neutral" ? "arrived" : tone})`;
}

function minutesUntil(iso: string | null | undefined, now: number): number | null {
  if (!iso) return null;
  const t = new Date(iso).getTime();
  if (Number.isNaN(t)) return null;
  return (t - now) / 60000;
}

/** Drop the STAR revision digit so OZZZI1 / OZZZI2 group as OZZZI. */
export function summaryGateName(gate: string | null | undefined): string | null {
  if (!gate) return null;
  const m = gate.toUpperCase().match(/^([A-Z]{3,5})\d[A-Z]?$/);
  return m ? m[1] : gate.toUpperCase();
}

/** Metered (non-arrived, non-excluded) flights grouped by gate, busiest first. */
function gateCounts(flights: FlowFlight[]): [string, number][] {
  const counts: Record<string, number> = {};
  for (const f of flights) {
    if (f.status === "arrived" || f.excluded) continue;
    const g = summaryGateName(f.gate);
    if (!g) continue;
    counts[g] = (counts[g] ?? 0) + 1;
  }
  return Object.entries(counts).sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
}

/** Gate → series token name (`series-1` … `series-8`), busiest gate first. */
function gateTokenMap(flights: FlowFlight[]): Record<string, string> {
  const map: Record<string, string> = {};
  gateCounts(flights).forEach(([g], i) => {
    map[g] = `series-${(i % 8) + 1}`;
  });
  return map;
}

function delayClass(min: number): string {
  if (min >= 15) return "text-level-over";
  if (min > 0) return "text-level-watch";
  return "text-ink-3";
}

// --- Summary ---

/** Share of inbound traffic by state, as one thin segmented meter. */
function StatusMeter({ flow }: { flow: Flow }) {
  const total = Math.max(flow.airborne + flow.ground + flow.proposed, 1);
  const seg = (n: number, cls: string) =>
    n > 0 ? <div className={cls} style={{ width: `${(n / total) * 100}%` }} /> : null;
  return (
    <div className="flex h-1.5 overflow-hidden rounded-full bg-chip" aria-hidden="true">
      {seg(flow.airborne, "bg-flight-airborne")}
      {seg(flow.ground, "bg-flight-ground")}
      {seg(flow.proposed, "bg-flight-proposed")}
    </div>
  );
}

function DemandRing({ flow }: { flow: Flow }) {
  const aar = flow.aar ?? 0;
  const pct = aar > 0 ? Math.min(100, Math.round((flow.demand_60min / aar) * 100)) : 0;
  const over = !!flow.over_capacity;

  return (
    <div className="flex items-center gap-5">
      <Donut
        size={112}
        thickness={0.2}
        label={aar > 0 ? `Demand ${pct}% of AAR` : `Demand ${flow.demand_60min}`}
        slices={
          aar > 0
            ? [
                { label: "Demand", value: pct, color: over ? "level-over" : "level-ok" },
                { label: "Headroom", value: 100 - pct, color: "chip" },
              ]
            : []
        }
        center={
          aar > 0 ? (
            <div className="flex flex-col items-center">
              <span className="font-mono text-xl font-bold">{pct}%</span>
              <span className="text-[10px] text-ink-3">of AAR</span>
            </div>
          ) : (
            <span className="font-mono text-2xl font-bold">{flow.demand_60min}</span>
          )
        }
      />
      <div className="min-w-0">
        <div className="text-xs text-ink-2">Demand / AAR · 60 min</div>
        <div className={`mt-1 font-mono text-2xl font-bold ${over ? "text-level-over" : ""}`}>
          {flow.demand_60min}
          {aar > 0 ? ` / ${aar}` : ""}
        </div>
        <div className="mt-1 max-w-xs text-sm text-ink-2">
          {aar <= 0
            ? "No program — set a rate on the TMU tab to meter this field."
            : over
              ? "Demand is above the acceptance rate — expect metering delays."
              : "Demand is within the acceptance rate."}
        </div>
      </div>
    </div>
  );
}

export function SummaryView({ flow }: { flow: Flow }) {
  const arrived = flow.flights.filter((f) => f.status === "arrived").length;
  return (
    <div className="flex flex-col gap-4">
      <div className="grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-5">
        <MetricCard
          className="col-span-2 md:col-span-3 xl:col-span-1"
          label="Inbound total"
          icon={PlaneLanding}
          value={flow.inbound}
          sub={<StatusMeter flow={flow} />}
        />
        <MetricCard label="Airborne" value={flow.airborne} tone="airborne" />
        <MetricCard label="Ground" value={flow.ground} tone="ground" />
        <MetricCard label="Proposed" value={flow.proposed} tone="proposed" />
        <MetricCard label="Arrived" value={arrived} tone="arrived" />
      </div>

      <div className="grid gap-4 lg:grid-cols-2">
        <Card className="p-4">
          <h2 className="mb-4 text-xl font-bold">Demand vs AAR</h2>
          <DemandRing flow={flow} />
        </Card>
        <GateBreakdown flow={flow} />
      </div>
    </div>
  );
}

function GateBreakdown({ flow }: { flow: Flow }) {
  const gates = useMemo(() => gateCounts(flow.flights), [flow.flights]);
  const tokens = useMemo(() => gateTokenMap(flow.flights), [flow.flights]);

  return (
    <Card className="p-4">
      <h2 className="text-xl font-bold">Aircraft by arrival gate</h2>
      <p className="mb-3 text-xs text-ink-3">STAR revisions grouped (PARCH3/4 → PARCH)</p>
      {gates.length === 0 ? (
        <EmptyState className="py-6">No gated arrivals in the metered stream right now.</EmptyState>
      ) : (
        <Bars
          horizontal
          label="Aircraft by arrival gate"
          data={gates}
          category={(d) => d[0]}
          value={(d) => d[1]}
          color={(d) => tokens[d[0]]}
          height={Math.max(96, gates.length * 28 + 32)}
          valueFormat={(v) => (Number.isInteger(v) ? String(v) : "")}
        />
      )}
    </Card>
  );
}

// --- Aircraft list ---

/** Nulls sort last, like the original hand-sorted table. */
const timeKey = (iso: string | null | undefined) => (iso ? new Date(iso).getTime() : Infinity);

/**
 * CFR cell. An *issued* CFR is a locked wheels-up actually given to a pilot (matching
 * the Departures/CFR page) — shown bold with a lock, green once it's due. A CFR that's
 * merely the scheduler's proposed slot renders faint, so controllers can tell them apart.
 */
function CfrCell({ f, now }: { f: FlowFlight; now: number }) {
  if (!f.cfr) return <span className="text-ink-3">—</span>;
  const imminent = new Date(f.cfr).getTime() <= now + 60000;
  if (f.cfr_issued) {
    return (
      <span
        className={`inline-flex items-center justify-end gap-1 font-semibold ${imminent ? "text-success" : "text-ink"}`}
        title="Call-for-release issued — locked wheels-up"
      >
        <Lock className="size-3" />
        {hhmmZulu(f.cfr)}
      </span>
    );
  }
  return (
    <span className="text-ink-3" title="Proposed wheels-up — auto-slotted, not yet issued">
      {hhmmZulu(f.cfr)}
    </span>
  );
}

function aircraftColumns(tokens: Record<string, string>, now: number): DataColumn<FlowFlight>[] {
  return [
    {
      id: "seq",
      accessorFn: (f) => f.seq ?? Infinity,
      header: "#",
      mono: true,
      align: "right",
      sortDescFirst: false,
      cell: (c) => <span className="text-ink-3">{c.row.original.seq ?? "—"}</span>,
    },
    {
      accessorKey: "callsign",
      header: "Callsign",
      icon: Plane,
      mono: true,
      cell: (c) => <span className="font-semibold">{c.row.original.callsign}</span>,
    },
    { id: "aircraft_type", accessorFn: (f) => f.aircraft_type ?? "", header: "Type", mono: true },
    { id: "dep", accessorFn: (f) => f.dep ?? "", header: "Dep", mono: true },
    {
      id: "gate",
      accessorFn: (f) => f.gate ?? "",
      header: "Gate",
      mono: true,
      cell: (c) => {
        const f = c.row.original;
        const token = f.gate ? tokens[summaryGateName(f.gate)!] : undefined;
        return <span style={token ? { color: `var(--${token})` } : undefined}>{f.gate ?? "—"}</span>;
      },
    },
    {
      accessorKey: "status",
      header: "Status",
      cell: (c) => (
        <StatusPill tone={toneOf("flight", c.row.original.status)}>{cap(c.row.original.status)}</StatusPill>
      ),
    },
    {
      id: "distance_nm",
      accessorFn: (f) => f.distance_nm ?? Infinity,
      header: "Dist",
      mono: true,
      align: "right",
      sortDescFirst: false,
      cell: (c) => (c.row.original.distance_nm == null ? "—" : Math.round(c.row.original.distance_nm)),
    },
    {
      id: "eta",
      accessorFn: (f) => timeKey(f.eta),
      header: "ETA",
      icon: Clock,
      mono: true,
      align: "right",
      sortDescFirst: false,
      cell: (c) => <span className="text-ink-2">{hhmmZulu(c.row.original.eta)}</span>,
    },
    {
      id: "sta",
      accessorFn: (f) => timeKey(f.sta),
      header: "STA",
      mono: true,
      align: "right",
      sortDescFirst: false,
      cell: (c) => hhmmZulu(c.row.original.sta),
    },
    {
      id: "delay_min",
      accessorFn: (f) => f.delay_min,
      header: "Delay",
      mono: true,
      align: "right",
      sortDescFirst: false,
      cell: (c) => {
        const d = c.row.original.delay_min;
        return <span className={delayClass(d)}>{d > 0 ? `+${d}` : "—"}</span>;
      },
    },
    {
      id: "cfr",
      accessorFn: (f) => timeKey(f.cfr),
      header: "CFR",
      icon: PlaneTakeoff,
      mono: true,
      align: "right",
      sortDescFirst: false,
      cell: (c) => <CfrCell f={c.row.original} now={now} />,
    },
  ];
}

export function AircraftView({ flow }: { flow: Flow }) {
  const tokens = useMemo(() => gateTokenMap(flow.flights), [flow.flights]);
  // `now` only drives the CFR "due" highlight; each flow refresh rebuilds the columns with a fresh one.
  const columns = useMemo(() => aircraftColumns(tokens, Date.now()), [tokens]);

  return (
    <div className="flex flex-col gap-2">
      <DataTable
        label={`Traffic filed to ${flow.icao}`}
        columns={columns}
        data={flow.flights}
        getRowId={(f) => f.callsign}
        initialSort={[{ id: "seq", desc: false }]}
        // Live ops list: always pages, never hides rows behind "Show all".
        rowCap={Infinity}
        pageSize={25}
        rowClassName={(f) => (f.excluded ? "opacity-45" : undefined)}
        empty={`No traffic filed to ${flow.icao} right now.`}
      />
      {flow.flights.some((f) => f.cfr_issued) && (
        <p className="flex items-center gap-1.5 text-xs text-ink-3">
          <Lock className="size-3" /> issued CFR — a locked wheels-up given to the pilot; faint times are
          proposed slots.
        </p>
      )}
    </div>
  );
}

// --- Arrival ladder ---

/** True if a flight passes the ladder's include-filters (each empty list = no constraint). */
function passesLadderFilters(f: FlowFlight, filters: LadderFilters | undefined): boolean {
  if (!filters) return true;
  if (filters.statuses?.length && !filters.statuses.includes(f.status)) return false;
  if (filters.gates?.length) {
    const g = summaryGateName(f.gate);
    if (!g || !filters.gates.includes(g)) return false;
  }
  if (filters.origins?.length && !filters.origins.some((o) => f.dep.toUpperCase().startsWith(o)))
    return false;
  if (filters.types?.length && !filters.types.some((t) => f.aircraft_type.toUpperCase().startsWith(t)))
    return false;
  return true;
}

const LADDER_CH = 7.5; // ≈ px per monospace char at text-xs

/** Estimated rendered pixel width of one arrival tag — connector + pill padding/border + dot + gaps +
 * text (callsign + "HHMMz" time + optional gate). */
function measureTagWidth(f: FlowFlight): number {
  const gate = f.gate ? String(f.gate) : "";
  const chars = f.callsign.length + 5 /* HHMMz */ + gate.length;
  const gaps = (gate ? 3 : 2) * 8; // gap-2 between the dot and the mono spans
  return 12 /* connector tick */ + 24 /* pill padding + border */ + 6 /* dot */ + gaps + chars * LADDER_CH;
}

export function LadderView({ flow, filters }: { flow: Flow; filters?: LadderFilters }) {
  const [win, setWin] = useState(60);
  const now = Date.now();
  const step = win <= 90 ? 10 : win <= 180 ? 15 : 30;
  const tokens = gateTokenMap(flow.flights);
  const colorOf = (f: FlowFlight) => {
    const gname = summaryGateName(f.gate);
    return gname && tokens[gname] ? `var(--${tokens[gname]})` : flightColor(f.status);
  };

  // Position by metered STA when available, else raw ETA.
  const timeOf = (f: FlowFlight) => f.sta ?? f.eta;
  const items = flow.flights
    .filter((f) => f.status !== "arrived" && !f.excluded && timeOf(f) && passesLadderFilters(f, filters))
    .map((f) => ({ key: f.callsign, min: minutesUntil(timeOf(f), now)!, time: timeOf(f) as string, data: f }));

  // Despite the name, this is "is any filter category active" — not literally "no match" (that's
  // only true once it's also combined with an empty `items`). Drives both the empty-state message
  // below and the header's filter indicator (see #96), since both need the same "filtered?" check.
  const noMatch =
    filters &&
    (filters.gates?.length || filters.statuses?.length || filters.origins?.length || filters.types?.length);

  return (
    <Card className="p-4">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <span className="flex items-center gap-1.5 text-xs text-ink-2">
          <span className="font-mono">{win} min</span> · {flow.aar != null ? "metered STA" : "ETA"} · now at
          bottom
          {noMatch && <Filter className="size-3.5 shrink-0" aria-label="Filtered" />}
        </span>
        <div className="flex gap-1">
          <Button
            size="sm"
            variant="outline"
            className="font-mono"
            disabled={win <= 30}
            onClick={() => setWin((w) => Math.max(30, w - 30))}
          >
            −30
          </Button>
          <Button
            size="sm"
            variant="outline"
            className="font-mono"
            disabled={win >= 240}
            onClick={() => setWin((w) => Math.min(240, w + 30))}
          >
            +30
          </Button>
        </div>
      </div>
      <ArrivalLadder
        items={items}
        now={now}
        win={win}
        pxPerMin={7}
        gutter={46}
        step={step}
        rowGap={26}
        minGap={13}
        pad={12}
        autoFitWidth
        emptyMessage={noMatch ? "No matching arrivals" : "No ETAs in window"}
        measureTagWidth={measureTagWidth}
        connectorColor={colorOf}
        renderTag={(f) => (
          <span
            className={`flex items-center gap-2 rounded-xs border border-line bg-panel-2 py-1 pl-2 pr-2.5 text-xs ${f.status === "proposed" ? "opacity-75" : ""}`}
          >
            <span className="size-1.5 shrink-0 rounded-full" style={{ background: colorOf(f) }} />
            <span className="font-mono font-semibold">{f.callsign}</span>
            <span className="font-mono text-ink-2">{hhmmZulu(timeOf(f))}</span>
            {f.gate && <span className="font-mono text-ink-3">{f.gate}</span>}
          </span>
        )}
      />
    </Card>
  );
}

// --- Demand vs AAR ---

export function DemandView({ flow }: { flow: Flow }) {
  const now = Date.now();
  const BIN = 15;
  const BINS = 8;
  const aar = flow.aar ?? 0;
  const binCap = aar > 0 ? Math.max(1, Math.round(aar / (60 / BIN))) : 0;

  const counts = new Array<number>(BINS).fill(0);
  for (const f of flow.flights) {
    if (f.status === "arrived" || f.excluded) continue;
    const m = minutesUntil(f.eta, now);
    if (m == null || m < 0 || m >= BINS * BIN) continue;
    counts[Math.floor(m / BIN)]++;
  }
  const bins = counts.map((n, i) => ({
    label: hhmmZulu(new Date(now + i * BIN * 60000).toISOString()),
    n,
    over: binCap > 0 && n > binCap,
  }));

  return (
    <Card className="p-4">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2 text-xs text-ink-2">
        <span>15 min bins · next {(BINS * BIN) / 60} h</span>
        {binCap > 0 && (
          <span className="font-mono text-warning">
            AAR {aar}/hr → cap {binCap}/bin
          </span>
        )}
      </div>
      <Bars
        label={`${flow.icao} arrival demand by 15-minute bin`}
        data={bins}
        category={(d) => d.label}
        value={(d) => d.n}
        color={(d) => (d.over ? "level-over" : "level-ok")}
        cap={binCap > 0 ? binCap : undefined}
        valueFormat={(v) => (Number.isInteger(v) ? String(v) : "")}
        height={200}
      />
      <p className="mt-3 text-sm text-ink-2">
        {aar > 0
          ? `Green is at or below capacity (${binCap}/bin from AAR ${aar}/hr). Red is over capacity.`
          : "No program — bars show raw arrival demand by ETA. Set an AAR on the TMU tab for capacity metering."}
      </p>
    </Card>
  );
}

// --- page shell ---

const SUBTITLE =
  "The whole picture for any airport — arrival demand and sequence, departures and CFRs, and live taxi-out times.";

export function AirportPage() {
  // `?icao=` (⌘K, deep links) preselects the airport.
  const initial = useSearch({ from: "/ops/airport" }).icao ?? "";
  const [query, setQuery] = useState(initial);
  const [icao, setIcao] = useState(initial);
  const [sub, setSub] = useState<Sub>("summary");
  const flow = useAirportFlow(icao);

  usePageHeader({ subtitle: SUBTITLE, title: icao ? `Airport · ${icao}` : undefined });

  function load() {
    const clean = query.replace(/[^a-zA-Z0-9]/g, "").toUpperCase();
    if (clean.length >= 3) setIcao(clean);
  }

  const tabs = [
    { value: "summary" as const, label: "Summary" },
    { value: "aircraft" as const, label: "Aircraft", count: flow.data?.flights.length },
    { value: "ladder" as const, label: "Ladder" },
    { value: "demand" as const, label: "Demand" },
    { value: "departures" as const, label: "Departures" },
    { value: "taxi" as const, label: "Taxi" },
  ];

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-2">
        <Input
          aria-label="Airport"
          className="w-28 font-mono uppercase"
          maxLength={4}
          placeholder="KJFK"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && load()}
        />
        <Button onClick={load}>Load</Button>
        {icao && flow.data && (
          <span className="ml-auto text-xs text-ink-3">
            {flow.isFetching ? "refreshing…" : "live · updates every 20s"}
          </span>
        )}
      </div>

      {!icao ? (
        <EmptyState icon={PlaneLanding} className="rounded-md border border-line py-12">
          Enter an arrival airport above to see its live flow.
        </EmptyState>
      ) : (
        <QueryState
          isLoading={!flow.data && !flow.isError}
          isError={flow.isError}
          loading={`Loading ${icao}…`}
          error={`Couldn't load flow for ${icao}.`}
          onRetry={() => flow.refetch()}
          className="rounded-md border border-line py-12"
        >
          {flow.data && (
            <>
              <Tabs value={sub} onChange={setSub} items={tabs} />
              {sub === "summary" && <SummaryView flow={flow.data} />}
              {sub === "aircraft" && <AircraftView flow={flow.data} />}
              {sub === "ladder" && <LadderView flow={flow.data} />}
              {sub === "demand" && <DemandView flow={flow.data} />}
              {sub === "departures" && <DeparturesView icao={icao} />}
              {sub === "taxi" && <TaxiView icao={icao} />}
            </>
          )}
        </QueryState>
      )}
    </div>
  );
}
