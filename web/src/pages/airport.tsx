import {useMemo, useState} from "react";
import {Button, Card, CardContent, Input} from "@ois/ui";

import {type Flow, type FlowFlight, useAirportFlow} from "@/lib/feed";
import {hhmmZulu} from "@/lib/time";

type Sub = "summary" | "aircraft" | "ladder" | "demand";

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

function SummaryView({ flow }: { flow: Flow }) {
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
      </CardContent>
    </Card>
  );
}

// --- Aircraft list ---

type ColKey = "callsign" | "aircraft_type" | "dep" | "status" | "distance_nm" | "groundspeed" | "eta";
const COLUMNS: { key: ColKey; label: string; num?: boolean; right?: boolean }[] = [
  { key: "callsign", label: "Callsign" },
  { key: "aircraft_type", label: "Type" },
  { key: "dep", label: "Dep" },
  { key: "status", label: "Status" },
  { key: "distance_nm", label: "Dist", num: true, right: true },
  { key: "groundspeed", label: "GS", num: true, right: true },
  { key: "eta", label: "ETA", right: true },
];

function AircraftView({ flow }: { flow: Flow }) {
  const [sortKey, setSortKey] = useState<ColKey>("eta");
  const [dir, setDir] = useState<1 | -1>(1);

  const rows = useMemo(() => {
    const val = (f: FlowFlight, k: ColKey): number | string => {
      const v = f[k];
      if (k === "eta") return v ? new Date(v as string).getTime() : Infinity;
      return v == null ? (typeof v === "number" ? Infinity : "") : (v as number | string);
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
                    <td className="py-1.5 pr-3 font-mono font-medium">{f.callsign}</td>
                    <td className="py-1.5 pr-3">{f.aircraft_type}</td>
                    <td className="py-1.5 pr-3 font-mono text-xs">{f.dep}</td>
                    <td className={`py-1.5 pr-3 ${st.text}`}>{st.label}</td>
                    <td className="py-1.5 pr-3 text-right tabular-nums">
                      {f.distance_nm == null ? "—" : Math.round(f.distance_nm)}
                    </td>
                    <td className="py-1.5 pr-3 text-right tabular-nums">
                      {f.groundspeed || "—"}
                    </td>
                    <td className="py-1.5 text-right font-mono text-xs tabular-nums">
                      {hhmmZulu(f.eta)}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      </CardContent>
    </Card>
  );
}

// --- Arrival ladder ---

function LadderView({ flow }: { flow: Flow }) {
  const [win, setWin] = useState(60);
  const now = Date.now();

  const PX = 7; // px per minute
  const ROW = 26; // min vertical spacing between adjacent tags
  const GUTTER = 62; // left column for the time axis
  const PAD = 12;
  const H = win * PX;
  const yOf = (min: number) => H - (Math.max(0, Math.min(min, win)) / win) * H;
  const step = win <= 90 ? 10 : win <= 180 ? 15 : 30;

  // Flights in window, earliest (nearest NOW) first — bottom to top.
  const items = flow.flights
    .filter((f) => f.status !== "arrived" && !f.excluded && f.eta)
    .map((f) => ({ f, min: minutesUntil(f.eta, now)! }))
    .filter((x) => x.min >= -1 && x.min <= win)
    .sort((a, b) => a.min - b.min);

  // Declutter: walk bottom→top, pushing each tag up so labels never overlap.
  let lastY = H + ROW;
  const placed = items.map(({ f, min }) => {
    const y = Math.min(yOf(min), lastY - ROW);
    lastY = y;
    return { f, min, y };
  });
  // Shift the whole stack down if decluttering pushed the top tag off-canvas.
  const topY = placed.length ? placed[placed.length - 1].y : H;
  const shift = topY < PAD ? PAD - topY : 0;
  const contentH = H + shift + PAD;

  const gridlines = [];
  for (let k = 0; k <= win / step; k++) {
    const min = k * step;
    const y = yOf(min) + shift;
    gridlines.push(
      <div key={`g${k}`}>
        <div
          className="absolute border-t border-border/40"
          style={{ top: y, left: GUTTER, right: 0 }}
        />
        <span
          className="absolute font-mono text-[10px] text-muted-foreground"
          style={{ top: y - 6, left: 0, width: GUTTER - 12, textAlign: "right" }}
        >
          {hhmmZulu(new Date(now + min * 60000).toISOString())}
        </span>
      </div>,
    );
  }

  return (
    <Card>
      <CardContent className="pt-6">
        <div className="mb-3 flex items-center justify-between">
          <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
            Arrival ladder · {win} min · now at bottom
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
        <div className="overflow-x-auto">
          <div className="relative" style={{ height: contentH, minWidth: 360 }}>
            {/* vertical time axis */}
            <div
              className="absolute top-0 bottom-0 border-l border-border/60"
              style={{ left: GUTTER }}
            />
            {gridlines}

            {/* NOW baseline */}
            <div
              className="absolute border-t-2 border-primary"
              style={{ top: yOf(0) + shift, left: GUTTER, right: 0 }}
            >
              <span
                className="absolute -top-2 text-[10px] font-semibold text-primary"
                style={{ left: 0, width: GUTTER - 12, textAlign: "right" }}
              >
                NOW
              </span>
            </div>

            {placed.length === 0 && (
              <div className="absolute inset-x-0 top-1/2 text-center text-sm text-muted-foreground">
                No ETAs in window
              </div>
            )}

            {placed.map(({ f, y }) => {
              const st = STATUS_STYLE[f.status] ?? STATUS_STYLE.arrived;
              return (
                <div
                  key={f.callsign}
                  className="absolute flex items-center"
                  style={{ top: y + shift - 11, left: GUTTER }}
                >
                  {/* connector tick to the axis */}
                  <span
                    className="h-0.5 w-3 shrink-0"
                    style={{ backgroundColor: st.color }}
                  />
                  <span
                    className={`flex items-center gap-2 rounded-md border border-border/70 bg-muted/40 py-1 pl-2 pr-2.5 text-xs ${f.status === "proposed" ? "opacity-75" : ""}`}
                    style={{ borderLeftWidth: 3, borderLeftColor: st.color }}
                  >
                    <span className="font-mono font-medium">{f.callsign}</span>
                    <span className="font-mono text-muted-foreground">
                      {hhmmZulu(f.eta)}
                    </span>
                  </span>
                </div>
              );
            })}
          </div>
        </div>
      </CardContent>
    </Card>
  );
}

// --- Demand vs AAR ---

function DemandView({ flow }: { flow: Flow }) {
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
          Live arrival picture for any airport — demand, aircraft, and sequence.
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
        </>
      )}
    </div>
  );
}
