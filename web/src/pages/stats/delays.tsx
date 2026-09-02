import {useState} from "react";
import {Badge, Card, CardContent, Switch} from "@ois/ui";
import {ArrowLeft} from "lucide-react";

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

const GREEN = "#22c55e";
const YELLOW = "#f59e0b";
const RED = "#ef4444";

/** Seconds → `M:SS`. */
function fmtDur(sec: number): string {
  const m = Math.floor(sec / 60);
  const s = sec % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

/**
 * Color a median. Normalized: relative to the airport baseline (≤1.1× green, ≤1.4× yellow, else red).
 * Fixed: green ≤20 min, yellow ≤40 min, red beyond.
 */
function colorFor(median: number, baseline: number | null, normalize: boolean): string {
  if (normalize && baseline && baseline > 0) {
    const r = median / baseline;
    return r <= 1.1 ? GREEN : r <= 1.4 ? YELLOW : RED;
  }
  return median <= 1200 ? GREEN : median <= 2400 ? YELLOW : RED;
}

function GroupList({
  groups,
  baseline,
  normalize,
  onPick,
}: {
  groups: DelayGroup[];
  baseline: number | null;
  normalize: boolean;
  onPick?: (key: string) => void;
}) {
  const max = Math.max(1, ...groups.map((g) => g.median_sec));
  return (
    <ul className="flex flex-col divide-y divide-border/60">
      {groups.map((g) => {
        const color = colorFor(g.median_sec, baseline, normalize);
        return (
          <li key={g.key} className="flex items-center gap-3 py-2 text-sm">
            {onPick ? (
              <button
                type="button"
                onClick={() => onPick(g.key)}
                className="w-16 shrink-0 text-left font-mono font-medium hover:text-primary"
              >
                {g.key}
              </button>
            ) : (
              <span className="w-16 shrink-0 font-mono font-medium">{g.key}</span>
            )}
            <div className="flex-1">
              <div
                className="h-2 rounded"
                style={{ width: `${(g.median_sec / max) * 100}%`, minWidth: 4, background: color }}
              />
            </div>
            <span className="w-12 shrink-0 text-right font-mono font-semibold" style={{ color }}>
              {fmtDur(g.median_sec)}
            </span>
            <span className="w-28 shrink-0 text-right text-xs tabular-nums text-muted-foreground">
              n {g.count} · p90 {fmtDur(g.p90_sec)}
            </span>
          </li>
        );
      })}
    </ul>
  );
}

const selectClass =
  "h-9 rounded-md border border-input bg-background px-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring";

export function DelaysPage() {
  const { data: me } = useMe();
  const canRead = hasPermission(me, "stats.data.read");

  const [kind, setKind] = useState<"departure" | "arrival">("departure");
  const [airport, setAirport] = useState("");
  const [runway, setRunway] = useState("");
  const [procedure, setProcedure] = useState("");
  const [hours, setHours] = useState(24);
  const [normalize, setNormalize] = useState(false);

  const summary = useDelaySummary({
    kind,
    airport: airport || undefined,
    runway: runway || undefined,
    procedure: procedure || undefined,
    hours,
  });

  if (!canRead) {
    return (
      <Card>
        <CardContent className="py-16 text-center text-sm text-muted-foreground">
          You don&apos;t have access to network statistics.
        </CardContent>
      </Card>
    );
  }

  const d = summary.data;
  const baseline = d?.overall.median_sec ?? null; // the airport's overall median (when filtered)
  const metric = kind === "departure" ? "taxi-out" : "arrival transit";

  const pickAirport = (a: string) => {
    setAirport(a);
    setRunway("");
    setProcedure("");
  };

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col gap-1">
        <h1 className="text-2xl font-semibold tracking-tight">Delays</h1>
        <p className="text-muted-foreground">
          Average {metric} times from live radar, over the last{" "}
          {WINDOWS.find((w) => w.h === hours)?.label ?? `${hours}h`}. Color by fixed thresholds, or
          normalized to each airport&apos;s baseline.
        </p>
      </div>

      {/* Controls */}
      <Card>
        <CardContent className="flex flex-wrap items-center gap-x-4 gap-y-3 pt-6">
          <div className="flex overflow-hidden rounded-md border">
            {(["departure", "arrival"] as const).map((k) => (
              <button
                key={k}
                type="button"
                onClick={() => setKind(k)}
                className={
                  "px-3 py-1.5 text-sm font-medium transition-colors " +
                  (kind === k ? "bg-primary text-primary-foreground" : "hover:bg-accent/40")
                }
              >
                {k === "departure" ? "Departures" : "Arrivals"}
              </button>
            ))}
          </div>

          <label className="flex items-center gap-1.5 text-xs text-muted-foreground">
            Window
            <select value={hours} onChange={(e) => setHours(Number(e.target.value))} className={selectClass}>
              {WINDOWS.map((w) => (
                <option key={w.h} value={w.h}>
                  {w.label}
                </option>
              ))}
            </select>
          </label>

          {airport ? (
            <>
              <button
                type="button"
                onClick={() => pickAirport("")}
                className="flex items-center gap-1 text-sm font-medium hover:text-primary"
              >
                <ArrowLeft className="size-4" /> All airports
              </button>
              <span className="font-mono font-semibold">{airport}</span>
              {d && d.by_runway.length > 0 && (
                <select value={runway} onChange={(e) => setRunway(e.target.value)} className={selectClass}>
                  <option value="">All runways</option>
                  {d.by_runway.map((g) => (
                    <option key={g.key} value={g.key}>
                      Rwy {g.key}
                    </option>
                  ))}
                </select>
              )}
              {d && d.by_procedure.length > 0 && (
                <select
                  value={procedure}
                  onChange={(e) => setProcedure(e.target.value)}
                  className={selectClass}
                >
                  <option value="">All {kind === "departure" ? "SIDs" : "STARs"}</option>
                  {d.by_procedure.map((g) => (
                    <option key={g.key} value={g.key}>
                      {g.key}
                    </option>
                  ))}
                </select>
              )}
            </>
          ) : (
            <span className="text-xs text-muted-foreground">Pick an airport below to drill in.</span>
          )}

          <label className="ml-auto flex items-center gap-1.5 text-xs text-muted-foreground">
            <Switch checked={normalize} onCheckedChange={setNormalize} className="scale-[0.68]" />
            Normalize per airport
          </label>
        </CardContent>
      </Card>

      {/* Overall */}
      {d && (
        <div className="flex flex-wrap items-center gap-x-6 gap-y-1 px-1 text-sm">
          <span>
            <span className="text-muted-foreground">Median </span>
            <span className="font-mono text-lg font-semibold">{fmtDur(d.overall.median_sec)}</span>
          </span>
          <span className="text-muted-foreground">avg {fmtDur(d.overall.avg_sec)}</span>
          <span className="text-muted-foreground">p90 {fmtDur(d.overall.p90_sec)}</span>
          <span className="text-muted-foreground">{d.overall.count} legs</span>
        </div>
      )}

      {/* Content */}
      {!d ? (
        <Card>
          <CardContent className="py-12 text-center text-sm text-muted-foreground">Loading…</CardContent>
        </Card>
      ) : d.overall.count === 0 ? (
        <Card>
          <CardContent className="py-12 text-center text-sm text-muted-foreground">
            No {metric} data in this window yet — the collector builds it from live traffic.
          </CardContent>
        </Card>
      ) : !airport ? (
        <Card>
          <CardContent className="pt-6">
            <div className="mb-2 text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
              By airport · median {metric}
            </div>
            <GroupList groups={d.by_airport} baseline={null} normalize={false} onPick={pickAirport} />
          </CardContent>
        </Card>
      ) : (
        <div className="grid gap-4 lg:grid-cols-2">
          <Card>
            <CardContent className="pt-6">
              <div className="mb-2 flex items-center justify-between">
                <span className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                  By runway
                </span>
                {normalize && <Badge variant="outline">vs {fmtDur(baseline ?? 0)}</Badge>}
              </div>
              {d.by_runway.length === 0 ? (
                <p className="py-6 text-center text-xs text-muted-foreground">No runway data.</p>
              ) : (
                <GroupList groups={d.by_runway} baseline={baseline} normalize={normalize} />
              )}
            </CardContent>
          </Card>
          <Card>
            <CardContent className="pt-6">
              <div className="mb-2 text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                By {kind === "departure" ? "SID" : "STAR"}
              </div>
              {d.by_procedure.length === 0 ? (
                <p className="py-6 text-center text-xs text-muted-foreground">No procedure data.</p>
              ) : (
                <GroupList groups={d.by_procedure} baseline={baseline} normalize={normalize} />
              )}
            </CardContent>
          </Card>
        </div>
      )}
    </div>
  );
}
