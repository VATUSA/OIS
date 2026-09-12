import {useMemo, useState} from "react";
import {Button, Card, CardContent, Input} from "@ois/ui";
import {Filter, Lock} from "lucide-react";

import type {LadderFilters} from "@/features/dashboard/types";
import {type Flow, type FlowFlight, useAirportFlow} from "@/lib/feed";
import {hhmmZulu} from "@/lib/time";
import {DeparturesView} from "@/pages/departures";
import {TaxiView} from "@/pages/taxi";
import {ArrivalLadder} from "@/components/ladder/ArrivalLadder";

type Sub = "summary" | "aircraft" | "ladder" | "demand" | "departures" | "taxi";

const STATUS_STYLE: Record<
  string,
  { dot: string; text: string; color: string; label: string }
> = {
  airborne: { dot: "bg-emerald-500", text: "text-emerald-500", color: "#10b981", label: "Airborne" },
  ground: { dot: "bg-amber-500", text: "text-amber-500", color: "#f59e0b", label: "Ground" },
  proposed: { dot: "bg-sky-500", text: "text-sky-500", color: "#0ea5e9", label: "Proposed" },
  arrived: { dot: "bg-muted-foreground", text: "text-muted-foreground", color: "#71717a", label: "Arrived" },
};

function minutesUntil(iso: string | null | undefined, now: number): number | null {
  if (!iso) return null;
  const t = new Date(iso).getTime();
  if (Number.isNaN(t)) return null;
  return (t - now) / 60000;
}

const GATE_PALETTE = [
  "#54b8e8",
  "#57d98a",
  "#f5a83d",
  "#c792ea",
  "#f07178",
  "#38bdf8",
  "#fbbf24",
];

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

function gateColorMap(flights: FlowFlight[]): Record<string, string> {
  const map: Record<string, string> = {};
  gateCounts(flights).forEach(([g], i) => {
    map[g] = GATE_PALETTE[i % GATE_PALETTE.length];
  });
  return map;
}

// --- Summary ---

function StatusBar({ flow }: { flow: Flow }) {
  const total = flow.airborne + flow.ground + flow.proposed;
  const seg = (n: number, cls: string) =>
    n > 0 ? (
      <div className={cls} style={{ width: `${(n / Math.max(total, 1)) * 100}%` }} />
    ) : null;
  return (
    <div className="flex h-2 overflow-hidden rounded-full bg-muted">
      {seg(flow.airborne, "bg-emerald-500")}
      {seg(flow.ground, "bg-amber-500")}
      {seg(flow.proposed, "bg-sky-500")}
    </div>
  );
}

function DemandRing({ flow }: { flow: Flow }) {
  const aar = flow.aar ?? 0;
  const pct = aar > 0 ? Math.min(100, Math.round((flow.demand_60min / aar) * 100)) : 0;
  const r = 46;
  const circ = 2 * Math.PI * r;
  const offset = circ * (1 - pct / 100);
  const over = !!flow.over_capacity;
  const color = over ? "text-destructive" : "text-emerald-500";

  return (
    <div className="flex items-center gap-4">
      <div className="relative size-28 shrink-0">
        <svg viewBox="0 0 120 120" className="size-full -rotate-90">
          <circle cx="60" cy="60" r={r} className="fill-none stroke-muted" strokeWidth="10" />
          {aar > 0 && (
            <circle
              cx="60"
              cy="60"
              r={r}
              className={`fill-none ${color} transition-all`}
              stroke="currentColor"
              strokeWidth="10"
              strokeLinecap="round"
              strokeDasharray={circ}
              strokeDashoffset={offset}
            />
          )}
        </svg>
        <div className="absolute inset-0 flex flex-col items-center justify-center">
          {aar > 0 ? (
            <>
              <span className="text-xl font-semibold">{pct}%</span>
              <span className="text-[10px] uppercase tracking-wide text-muted-foreground">
                of AAR
              </span>
            </>
          ) : (
            <span className="text-3xl font-semibold">{flow.demand_60min}</span>
          )}
        </div>
      </div>
      <div>
        <div className="text-xs uppercase tracking-wide text-muted-foreground">
          Demand / AAR · 60 min
        </div>
        <div className={`text-2xl font-semibold ${over ? "text-destructive" : ""}`}>
          {flow.demand_60min}
          {aar > 0 ? ` / ${aar}` : ""}
        </div>
        <div className="mt-1 max-w-xs text-sm text-muted-foreground">
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

function Metric({ label, value, cls }: { label: string; value: number; cls: string }) {
  return (
    <div className="flex flex-col gap-1">
      <span className="text-xs uppercase tracking-wide text-muted-foreground">{label}</span>
      <span className={`text-2xl font-semibold ${cls}`}>{value}</span>
    </div>
  );
}

export function SummaryView({ flow }: { flow: Flow }) {
  return (
    <Card>
      <CardContent className="flex flex-col gap-6 pt-6">
        <div className="flex items-end justify-between">
          <div>
            <div className="font-mono text-2xl font-semibold">{flow.icao}</div>
            <div className="text-sm text-muted-foreground">Arrival summary</div>
          </div>
          <div className="text-right">
            <div className="text-3xl font-semibold">{flow.inbound}</div>
            <div className="text-xs uppercase tracking-wide text-muted-foreground">
              Inbound total
            </div>
          </div>
        </div>

        <StatusBar flow={flow} />

        <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
          <Metric label="Airborne" value={flow.airborne} cls="text-emerald-500" />
          <Metric label="Ground" value={flow.ground} cls="text-amber-500" />
          <Metric label="Proposed" value={flow.proposed} cls="text-sky-500" />
          <Metric
            label="Arrived"
            value={flow.flights.filter((f) => f.status === "arrived").length}
            cls="text-muted-foreground"
          />
        </div>

        <div className="border-t pt-6">
          <DemandRing flow={flow} />
        </div>

        <GateBreakdown flow={flow} />
      </CardContent>
    </Card>
  );
}

function GateBreakdown({ flow }: { flow: Flow }) {
  const gates = gateCounts(flow.flights);
  const colors = gateColorMap(flow.flights);
  const max = gates.length ? gates[0][1] : 1;

  return (
    <div className="border-t pt-6">
      <div className="mb-3 text-xs font-medium uppercase tracking-wide text-muted-foreground">
        Aircraft by arrival gate
        <span className="ml-2 normal-case text-muted-foreground/70">
          · STAR revisions grouped (PARCH3/4 → PARCH)
        </span>
      </div>
      {gates.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          No gated arrivals in the metered stream right now.
        </p>
      ) : (
        <div className="flex flex-col gap-2">
          {gates.map(([g, n]) => (
            <div key={g} className="flex items-center gap-3">
              <span
                className="w-16 shrink-0 font-mono text-sm font-medium"
                style={{ color: colors[g] }}
              >
                {g}
              </span>
              <div className="h-2 flex-1 overflow-hidden rounded-full bg-muted">
                <div
                  className="h-full rounded-full"
                  style={{ width: `${(n / max) * 100}%`, backgroundColor: colors[g] }}
                />
              </div>
              <span className="w-6 shrink-0 text-right text-sm tabular-nums">{n}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// --- Aircraft list ---

type ColKey =
  | "seq"
  | "callsign"
  | "aircraft_type"
  | "dep"
  | "gate"
  | "status"
  | "distance_nm"
  | "eta"
  | "sta"
  | "delay_min"
  | "cfr";
const COLUMNS: { key: ColKey; label: string; num?: boolean; right?: boolean }[] = [
  { key: "seq", label: "#", num: true },
  { key: "callsign", label: "Callsign" },
  { key: "aircraft_type", label: "Type" },
  { key: "dep", label: "Dep" },
  { key: "gate", label: "Gate" },
  { key: "status", label: "Status" },
  { key: "distance_nm", label: "Dist", num: true, right: true },
  { key: "eta", label: "ETA", right: true },
  { key: "sta", label: "STA", right: true },
  { key: "delay_min", label: "Delay", num: true, right: true },
  { key: "cfr", label: "CFR", right: true },
];
const DATE_KEYS = new Set<ColKey>(["eta", "sta", "cfr"]);
const NUM_KEYS = new Set<ColKey>(["seq", "distance_nm", "delay_min"]);

function delayClass(min: number): string {
  if (min >= 15) return "text-destructive";
  if (min > 0) return "text-amber-500";
  return "text-muted-foreground";
}

/**
 * CFR cell. An *issued* CFR is a locked wheels-up actually given to a pilot (matching
 * the Departures/CFR page) — shown bold with a lock, green once it's due. A CFR that's
 * merely the scheduler's proposed slot renders faint, so controllers can tell them apart.
 */
function CfrCell({ f, now }: { f: FlowFlight; now: number }) {
  if (!f.cfr) {
    return (
      <td className="py-1.5 text-right font-mono text-xs tabular-nums text-muted-foreground">
        —
      </td>
    );
  }
  const imminent = new Date(f.cfr).getTime() <= now + 60000;
  if (f.cfr_issued) {
    return (
      <td className="py-1.5 text-right font-mono text-xs tabular-nums">
        <span
          className={`inline-flex items-center justify-end gap-1 font-semibold ${imminent ? "text-emerald-500" : "text-foreground"}`}
          title="Call-for-release issued — locked wheels-up"
        >
          <Lock className="size-3" />
          {hhmmZulu(f.cfr)}
        </span>
      </td>
    );
  }
  return (
    <td className="py-1.5 text-right font-mono text-xs tabular-nums">
      <span
        className="text-muted-foreground/60"
        title="Proposed wheels-up — auto-slotted, not yet issued"
      >
        {hhmmZulu(f.cfr)}
      </span>
    </td>
  );
}

export function AircraftView({ flow }: { flow: Flow }) {
  const [sortKey, setSortKey] = useState<ColKey>("seq");
  const [dir, setDir] = useState<1 | -1>(1);
  const now = Date.now();

  const rows = useMemo(() => {
    const val = (f: FlowFlight, k: ColKey): number | string => {
      const v = f[k];
      if (DATE_KEYS.has(k)) return v == null ? Infinity : new Date(v as string).getTime();
      if (NUM_KEYS.has(k)) return v == null ? Infinity : (v as number);
      return v == null ? "" : (v as string);
    };
    return [...flow.flights].sort((a, b) => {
      const va = val(a, sortKey);
      const vb = val(b, sortKey);
      if (va < vb) return -1 * dir;
      if (va > vb) return 1 * dir;
      return 0;
    });
  }, [flow.flights, sortKey, dir]);

  function clickSort(k: ColKey) {
    if (k === sortKey) setDir((d) => (d === 1 ? -1 : 1));
    else {
      setSortKey(k);
      setDir(1);
    }
  }

  if (!flow.flights.length) {
    return (
      <Card>
        <CardContent className="py-10 text-center text-sm text-muted-foreground">
          No traffic filed to {flow.icao} right now.
        </CardContent>
      </Card>
    );
  }

  const gateColors = gateColorMap(flow.flights);

  return (
    <Card>
      <CardContent className="pt-6">
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr className="text-xs uppercase tracking-wide text-muted-foreground">
                {COLUMNS.map((c) => (
                  <th
                    key={c.key}
                    onClick={() => clickSort(c.key)}
                    className={`cursor-pointer select-none pb-2 pr-3 font-medium ${c.right ? "text-right" : "text-left"} ${c.key === sortKey ? "text-foreground" : ""}`}
                  >
                    {c.label}
                    {c.key === sortKey ? (dir === 1 ? " ▴" : " ▾") : ""}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((f) => {
                const st = STATUS_STYLE[f.status] ?? STATUS_STYLE.arrived;
                return (
                  <tr
                    key={f.callsign}
                    className={`border-t ${f.excluded ? "opacity-45" : ""}`}
                  >
                    <td className="py-1.5 pr-3 text-right tabular-nums text-muted-foreground">
                      {f.seq ?? "—"}
                    </td>
                    <td className="py-1.5 pr-3 font-mono font-medium">{f.callsign}</td>
                    <td className="py-1.5 pr-3">{f.aircraft_type}</td>
                    <td className="py-1.5 pr-3 font-mono text-xs">{f.dep}</td>
                    <td
                      className="py-1.5 pr-3 font-mono text-xs"
                      style={{ color: f.gate ? gateColors[summaryGateName(f.gate)!] : undefined }}
                    >
                      {f.gate ?? "—"}
                    </td>
                    <td className={`py-1.5 pr-3 ${st.text}`}>{st.label}</td>
                    <td className="py-1.5 pr-3 text-right tabular-nums">
                      {f.distance_nm == null ? "—" : Math.round(f.distance_nm)}
                    </td>
                    <td className="py-1.5 pr-3 text-right font-mono text-xs tabular-nums text-muted-foreground">
                      {hhmmZulu(f.eta)}
                    </td>
                    <td className="py-1.5 pr-3 text-right font-mono text-xs tabular-nums">
                      {hhmmZulu(f.sta)}
                    </td>
                    <td
                      className={`py-1.5 pr-3 text-right tabular-nums ${delayClass(f.delay_min)}`}
                    >
                      {f.delay_min > 0 ? `+${f.delay_min}` : "—"}
                    </td>
                    <CfrCell f={f} now={now} />
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
        {flow.flights.some((f) => f.cfr_issued) && (
          <p className="mt-3 flex items-center gap-1.5 text-xs text-muted-foreground">
            <Lock className="size-3" /> issued CFR — a locked wheels-up given to the
            pilot; faint times are proposed slots.
          </p>
        )}
      </CardContent>
    </Card>
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

/** Estimated rendered pixel width of one arrival tag — connector + pill padding/border/gaps +
 * text (callsign + "HHMMz" time + optional gate). */
function measureTagWidth(f: FlowFlight): number {
  const gate = f.gate ? String(f.gate) : "";
  const chars = f.callsign.length + 5 /* HHMMz */ + gate.length;
  const gaps = (gate ? 2 : 1) * 8; // gap-2 between the mono spans
  return 12 /* connector tick */ + 24 /* pill padding + border */ + gaps + chars * LADDER_CH;
}

export function LadderView({ flow, filters }: { flow: Flow; filters?: LadderFilters }) {
  const [win, setWin] = useState(60);
  const now = Date.now();
  const step = win <= 90 ? 10 : win <= 180 ? 15 : 30;
  const gateColors = gateColorMap(flow.flights);

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
    <Card>
      <CardContent className="pt-6">
        <div className="mb-3 flex items-center justify-between">
          <span className="flex items-center gap-1.5 text-xs font-medium uppercase tracking-wide text-muted-foreground">
            Arrival ladder · {win} min · {flow.aar != null ? "metered STA" : "ETA"} · now
            at bottom
            {noMatch && <Filter className="size-3.5 shrink-0" />}
          </span>
          <div className="flex gap-1">
            <Button
              size="sm"
              variant="secondary"
              disabled={win <= 30}
              onClick={() => setWin((w) => Math.max(30, w - 30))}
            >
              −30
            </Button>
            <Button
              size="sm"
              variant="secondary"
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
          connectorColor={(f) => {
            const st = STATUS_STYLE[f.status] ?? STATUS_STYLE.arrived;
            const gname = summaryGateName(f.gate);
            return (gname && gateColors[gname]) || st.color;
          }}
          renderTag={(f) => {
            const st = STATUS_STYLE[f.status] ?? STATUS_STYLE.arrived;
            const gname = summaryGateName(f.gate);
            const color = (gname && gateColors[gname]) || st.color;
            return (
              <span
                className={`flex items-center gap-2 rounded-md border border-border/70 bg-muted/40 py-1 pl-2 pr-2.5 text-xs ${f.status === "proposed" ? "opacity-75" : ""}`}
                style={{ borderLeftWidth: 3, borderLeftColor: color }}
              >
                <span className="font-mono font-medium">{f.callsign}</span>
                <span className="font-mono text-muted-foreground">{hhmmZulu(timeOf(f))}</span>
                {f.gate && <span className="font-mono text-muted-foreground/80">{f.gate}</span>}
              </span>
            );
          }}
        />
      </CardContent>
    </Card>
  );
}

// --- Demand vs AAR ---

export function DemandView({ flow }: { flow: Flow }) {
  const now = Date.now();
  const BIN = 15;
  const BINS = 8;
  const aar = flow.aar ?? 0;
  const cap = aar > 0 ? Math.max(1, Math.round(aar / (60 / BIN))) : 0;

  const counts = new Array(BINS).fill(0);
  for (const f of flow.flights) {
    if (f.status === "arrived" || f.excluded) continue;
    const m = minutesUntil(f.eta, now);
    if (m == null || m < 0 || m >= BINS * BIN) continue;
    counts[Math.floor(m / BIN)]++;
  }
  const max = Math.max(...counts, cap, 1);
  const H = 160;

  return (
    <Card>
      <CardContent className="pt-6">
        <div className="mb-4 text-xs font-medium uppercase tracking-wide text-muted-foreground">
          Demand vs AAR — 15 min bins · next {(BINS * BIN) / 60} h
        </div>
        <div className="relative overflow-x-auto">
          <div className="flex items-end gap-2" style={{ height: H }}>
            {counts.map((n, i) => {
              const over = cap > 0 && n > cap;
              return (
                <div key={i} className="flex flex-1 flex-col items-center justify-end gap-1">
                  <span className="text-xs tabular-nums text-muted-foreground">{n || ""}</span>
                  <div
                    className={`w-full rounded-t ${over ? "bg-destructive" : "bg-emerald-500/80"}`}
                    style={{ height: Math.max(2, (n / max) * (H - 24)) }}
                  />
                </div>
              );
            })}
          </div>
          {cap > 0 && (
            <div
              className="pointer-events-none absolute inset-x-0 border-t-2 border-dashed border-amber-400"
              style={{ bottom: 24 + (cap / max) * (H - 24) }}
            >
              <span className="absolute -top-4 right-0 text-[10px] font-medium text-amber-400">
                cap {cap}/bin
              </span>
            </div>
          )}
        </div>
        <div className="mt-2 flex gap-2 text-[10px] font-mono text-muted-foreground">
          {counts.map((_, i) => (
            <span key={i} className="flex-1 text-center">
              {hhmmZulu(new Date(now + i * BIN * 60000).toISOString())}
            </span>
          ))}
        </div>
        <p className="mt-3 text-sm text-muted-foreground">
          {aar > 0
            ? `Olive is at or below capacity (${cap}/bin from AAR ${aar}/hr). Red is over capacity.`
            : "No program — bars show raw arrival demand by ETA. Set an AAR on the TMU tab for capacity metering."}
        </p>
      </CardContent>
    </Card>
  );
}

// --- page shell ---

const SUBS: { id: Sub; label: string }[] = [
  { id: "summary", label: "Summary" },
  { id: "aircraft", label: "Aircraft list" },
  { id: "ladder", label: "Arrival ladder" },
  { id: "demand", label: "Demand vs AAR" },
  { id: "departures", label: "Departures" },
  { id: "taxi", label: "Taxi" },
];

export function AirportPage() {
  const [query, setQuery] = useState("");
  const [icao, setIcao] = useState("");
  const [sub, setSub] = useState<Sub>("summary");
  const flow = useAirportFlow(icao);

  function load() {
    const clean = query.replace(/[^a-zA-Z0-9]/g, "").toUpperCase();
    if (clean.length >= 3) setIcao(clean);
  }

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Airport dashboard</h1>
        <p className="text-muted-foreground">
          The whole picture for any airport — arrival demand and sequence, departures and
          CFRs, and live taxi-out times.
        </p>
      </div>

      <Card>
        <CardContent className="flex flex-wrap items-end gap-3 pt-6">
          <label className="flex flex-col gap-1 text-xs font-medium uppercase tracking-wide text-muted-foreground">
            Airport
            <Input
              className="w-32 font-mono uppercase"
              maxLength={4}
              placeholder="KJFK"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && load()}
            />
          </label>
          <Button onClick={load}>Load</Button>
          {icao && flow.data && (
            <span className="ml-auto text-xs text-muted-foreground">
              {flow.isFetching ? "refreshing…" : "live · updates every 20s"}
            </span>
          )}
        </CardContent>
      </Card>

      {!icao ? (
        <Card>
          <CardContent className="py-12 text-center text-sm text-muted-foreground">
            Enter an arrival airport above to see its live flow.
          </CardContent>
        </Card>
      ) : flow.isError ? (
        <Card>
          <CardContent className="py-12 text-center text-sm text-muted-foreground">
            Couldn&apos;t load flow for {icao}.
          </CardContent>
        </Card>
      ) : !flow.data ? (
        <Card>
          <CardContent className="py-12 text-center text-sm text-muted-foreground">
            Loading {icao}…
          </CardContent>
        </Card>
      ) : (
        <>
          <div className="flex gap-1 border-b">
            {SUBS.map((s) => (
              <button
                key={s.id}
                type="button"
                onClick={() => setSub(s.id)}
                className={
                  "-mb-px border-b-2 px-4 py-2 text-sm font-medium transition-colors " +
                  (sub === s.id
                    ? "border-primary text-foreground"
                    : "border-transparent text-muted-foreground hover:text-foreground")
                }
              >
                {s.label}
              </button>
            ))}
          </div>
          {sub === "summary" && <SummaryView flow={flow.data} />}
          {sub === "aircraft" && <AircraftView flow={flow.data} />}
          {sub === "ladder" && <LadderView flow={flow.data} />}
          {sub === "demand" && <DemandView flow={flow.data} />}
          {sub === "departures" && <DeparturesView icao={icao} />}
          {sub === "taxi" && <TaxiView icao={icao} />}
        </>
      )}
    </div>
  );
}
