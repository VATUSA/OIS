import {useMemo, useState} from "react";
import {Badge, Button, buttonVariants, Card, CardContent, Input} from "@ois/ui";
import {Link} from "@tanstack/react-router";
import {ArrowDownToLine, ArrowUpFromLine, Film, Plane, TrendingUp} from "lucide-react";

import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {
  type NetworkPoint,
  useAirportMovements,
  useAirportsTop,
  useAirportStats,
  useCaptures,
  useNetworkHistory,
} from "@/lib/stats";
import {formatZulu, formatZuluFull} from "@/lib/time";

const RANGES = [
  { id: "24h", label: "24h", days: 1 },
  { id: "7d", label: "7d", days: 7 },
  { id: "30d", label: "30d", days: 30 },
] as const;
type RangeId = (typeof RANGES)[number]["id"];

const normIcao = (s: string) => s.replace(/[^a-zA-Z0-9]/g, "").toUpperCase().slice(0, 4);

/** Compact inline SVG: avg-pilots area + peak-pilots line over the window. */
function NetworkChart({ points }: { points: NetworkPoint[] }) {
  const W = 820;
  const H = 220;
  const PAD = 28;

  const path = useMemo(() => {
    if (points.length < 2) return null;
    const maxY = Math.max(1, ...points.map((p) => p.peak_pilots ?? p.avg_pilots ?? 0));
    const n = points.length;
    const x = (i: number) => PAD + (i / (n - 1)) * (W - 2 * PAD);
    const y = (v: number) => H - PAD - (v / maxY) * (H - 2 * PAD);
    const avg = points.map((p, i) => `${x(i)},${y(p.avg_pilots ?? 0)}`);
    const peak = points.map((p, i) => `${x(i)},${y(p.peak_pilots ?? 0)}`);
    const area = `M ${x(0)},${H - PAD} L ${avg.join(" L ")} L ${x(n - 1)},${H - PAD} Z`;
    return { area, avgLine: `M ${avg.join(" L ")}`, peakLine: `M ${peak.join(" L ")}`, maxY };
  }, [points]);

  if (!path)
    return (
      <div className="flex h-40 items-center justify-center text-sm text-muted-foreground">
        Not enough data yet — collection accrues from now forward.
      </div>
    );

  const first = points[0]?.hour;
  const last = points[points.length - 1]?.hour;

  return (
    <div className="flex flex-col gap-1">
      <svg viewBox={`0 0 ${W} ${H}`} className="w-full" role="img" aria-label="Network pilots over time">
        <line x1={PAD} y1={H - PAD} x2={W - PAD} y2={H - PAD} className="stroke-border" strokeWidth={1} />
        <path d={path.area} className="fill-primary/15" />
        <path d={path.peakLine} className="stroke-primary/40" strokeWidth={1.5} fill="none" />
        <path d={path.avgLine} className="stroke-primary" strokeWidth={2} fill="none" />
        <text x={PAD} y={PAD - 10} className="fill-muted-foreground text-[11px]">
          peak {path.maxY}
        </text>
      </svg>
      <div className="flex justify-between px-6 text-[11px] text-muted-foreground">
        <span>{first ? formatZulu(first) : ""}</span>
        <span>avg pilots (solid) · peak (faint)</span>
        <span>{last ? formatZulu(last) : ""}</span>
      </div>
    </div>
  );
}

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
    <Card>
      <CardContent className="flex flex-col gap-4 pt-6">
        <div className="flex items-center gap-2">
          <span className="flex size-8 items-center justify-center rounded-md bg-primary/10 text-primary">
            <Plane className="size-4" />
          </span>
          <div className="flex flex-col">
            <span className="font-semibold">Airport activity</span>
            <span className="text-xs text-muted-foreground">
              All-time recorded departures and arrivals for a US airport.
            </span>
          </div>
        </div>

        <form className="flex items-end gap-2" onSubmit={submit}>
          <Input
            className="w-40 font-mono uppercase"
            value={entry}
            onChange={(e) => setEntry(normIcao(e.target.value))}
            placeholder="KATL"
          />
          <Button type="submit" disabled={normIcao(entry).length < 3}>
            Look up
          </Button>
        </form>

        {!icao ? (
          <p className="py-2 text-sm text-muted-foreground">
            Enter an ICAO, or pick one from the busiest list.
          </p>
        ) : !s ? (
          <p className="py-2 text-sm text-muted-foreground">Loading {icao}…</p>
        ) : (
          <div className="flex flex-col gap-4">
            <div className="flex flex-wrap gap-3">
              <div className="flex flex-col rounded-md border bg-muted/20 px-4 py-2">
                <span className="flex items-center gap-1 text-xs text-muted-foreground">
                  <ArrowUpFromLine className="size-3" /> Departures
                </span>
                <span className="text-xl font-semibold tabular-nums">{s.departures}</span>
              </div>
              <div className="flex flex-col rounded-md border bg-muted/20 px-4 py-2">
                <span className="flex items-center gap-1 text-xs text-muted-foreground">
                  <ArrowDownToLine className="size-3" /> Arrivals
                </span>
                <span className="text-xl font-semibold tabular-nums">{s.arrivals}</span>
              </div>
            </div>

            {s.top_aircraft.length > 0 && (
              <div>
                <h3 className="mb-1.5 text-sm font-medium">Top aircraft</h3>
                <div className="flex flex-wrap gap-1.5">
                  {s.top_aircraft.map((a) => (
                    <Badge key={a.key ?? "?"} variant="secondary" className="gap-1 font-mono">
                      {a.key ?? "?"}
                      <span className="text-muted-foreground">{a.count}</span>
                    </Badge>
                  ))}
                </div>
              </div>
            )}

            <div className="grid gap-4 sm:grid-cols-2">
              <TopList title="Top destinations" rows={s.top_destinations} onPick={onPick} />
              <TopList title="Top origins" rows={s.top_origins} onPick={onPick} />
            </div>

            <div>
              <div className="mb-2 flex items-center gap-2">
                <h3 className="text-sm font-medium">Recent movements</h3>
                <div className="flex gap-1">
                  <Button
                    size="sm"
                    variant={dir === "arr" ? "default" : "secondary"}
                    className="h-7 px-2"
                    onClick={() => setDir("arr")}
                  >
                    Arrivals
                  </Button>
                  <Button
                    size="sm"
                    variant={dir === "dep" ? "default" : "secondary"}
                    className="h-7 px-2"
                    onClick={() => setDir("dep")}
                  >
                    Departures
                  </Button>
                </div>
              </div>
              {(movements.data?.length ?? 0) === 0 ? (
                <p className="text-sm text-muted-foreground">No recent {dir === "arr" ? "arrivals" : "departures"}.</p>
              ) : (
                <div className="overflow-x-auto">
                  <table className="w-full text-sm">
                    <thead>
                      <tr className="text-left text-xs uppercase tracking-wide text-muted-foreground">
                        <th className="pb-1 pr-3 font-medium">Callsign</th>
                        <th className="pb-1 pr-3 font-medium">{dir === "arr" ? "From" : "To"}</th>
                        <th className="pb-1 pr-3 font-medium">Type</th>
                        <th className="pb-1 font-medium">Logon</th>
                      </tr>
                    </thead>
                    <tbody>
                      {movements.data!.map((m) => (
                        <tr key={String(m.session_id)} className="border-t">
                          <td className="py-1.5 pr-3 font-mono font-medium">
                            <Link
                              to="/historical/flights/$flightId"
                              params={{ flightId: String(m.session_id) }}
                              className="hover:underline"
                            >
                              {m.callsign}
                            </Link>
                          </td>
                          <td className="py-1.5 pr-3 font-mono text-muted-foreground">
                            {dir === "arr" ? m.departure ?? "—" : m.arrival ?? "—"}
                          </td>
                          <td className="py-1.5 pr-3 font-mono">{m.aircraft_short ?? "—"}</td>
                          <td className="py-1.5 text-muted-foreground">{formatZulu(m.logon_time)}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function TopList({
  title,
  rows,
  onPick,
}: {
  title: string;
  rows: { key?: string | null; count: number }[];
  onPick: (icao: string) => void;
}) {
  return (
    <div>
      <h3 className="mb-1.5 text-sm font-medium">{title}</h3>
      {rows.length === 0 ? (
        <p className="text-sm text-muted-foreground">No data.</p>
      ) : (
        <ul className="flex flex-col gap-0.5 text-sm">
          {rows.map((r) => (
            <li key={r.key ?? "?"} className="flex items-center justify-between gap-2">
              <button
                type="button"
                className="font-mono hover:underline"
                onClick={() => r.key && onPick(r.key)}
              >
                {r.key ?? "?"}
              </button>
              <span className="tabular-nums text-muted-foreground">{r.count}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

export function StatsPage() {
  const { data: me } = useMe();
  const canRead = hasPermission(me, "stats.data.read");
  const [range, setRange] = useState<RangeId>("7d");
  const [icao, setIcao] = useState<string | null>(null);

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
    return (
      <Card>
        <CardContent className="py-16 text-center text-sm text-muted-foreground">
          You don&apos;t have access to network statistics.
        </CardContent>
      </Card>
    );
  }

  return (
    <div className="flex w-full flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Network statistics</h1>
        <p className="text-muted-foreground">
          Historical VATSIM activity collected from the live feed (US-relevant traffic + saved
          event captures).
        </p>
      </div>

      <Card>
        <CardContent className="flex flex-col gap-3 pt-6">
          <div className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-2">
              <span className="flex size-8 items-center justify-center rounded-md bg-primary/10 text-primary">
                <TrendingUp className="size-4" />
              </span>
              <span className="font-semibold">Pilots online</span>
            </div>
            <div className="flex gap-1">
              {RANGES.map((r) => (
                <Button
                  key={r.id}
                  size="sm"
                  variant={range === r.id ? "default" : "secondary"}
                  className="h-7 px-2"
                  onClick={() => setRange(r.id)}
                >
                  {r.label}
                </Button>
              ))}
            </div>
          </div>
          {!history.data ? (
            <div className="flex h-40 items-center justify-center text-sm text-muted-foreground">
              Loading…
            </div>
          ) : (
            <NetworkChart points={history.data} />
          )}
        </CardContent>
      </Card>

      <div className="grid gap-6 lg:grid-cols-[320px_1fr]">
        <Card>
          <CardContent className="flex flex-col gap-3 pt-6">
            <span className="font-semibold">Busiest airports</span>
            {!top.data ? (
              <p className="text-sm text-muted-foreground">Loading…</p>
            ) : top.data.length === 0 ? (
              <p className="text-sm text-muted-foreground">No data yet.</p>
            ) : (
              <ol className="flex flex-col gap-0.5 text-sm">
                {top.data.map((a, i) => (
                  <li key={a.key ?? i} className="flex items-center justify-between gap-2">
                    <button
                      type="button"
                      className="flex items-center gap-2 hover:underline"
                      onClick={() => a.key && setIcao(a.key)}
                    >
                      <span className="w-5 text-right tabular-nums text-muted-foreground">{i + 1}</span>
                      <span className="font-mono font-medium">{a.key ?? "?"}</span>
                    </button>
                    <span className="tabular-nums text-muted-foreground">{a.count}</span>
                  </li>
                ))}
              </ol>
            )}
          </CardContent>
        </Card>

        <AirportLookup icao={icao} onPick={setIcao} />
      </div>

      <SavedCaptures />
    </div>
  );
}

function SavedCaptures() {
  const captures = useCaptures();
  const rows = captures.data ?? [];
  if (captures.data && rows.length === 0) return null;

  return (
    <Card>
      <CardContent className="flex flex-col gap-3 pt-6">
        <div className="flex items-center gap-2">
          <span className="flex size-8 items-center justify-center rounded-md bg-primary/10 text-primary">
            <Film className="size-4" />
          </span>
          <div className="flex flex-col">
            <span className="font-semibold">Saved captures</span>
            <span className="text-xs text-muted-foreground">
              Replay a recorded event or window on the map.
            </span>
          </div>
        </div>
        {!captures.data ? (
          <p className="text-sm text-muted-foreground">Loading…</p>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-xs uppercase tracking-wide text-muted-foreground">
                  <th className="pb-2 pr-3 font-medium">Capture</th>
                  <th className="pb-2 pr-3 font-medium">Window</th>
                  <th className="pb-2 pr-3 font-medium">Status</th>
                  <th className="pb-2" />
                </tr>
              </thead>
              <tbody>
                {rows.map((c) => (
                  <tr key={c.id} className="border-t">
                    <td className="py-2 pr-3">{c.event_title || c.label || "Capture"}</td>
                    <td className="py-2 pr-3 text-muted-foreground">
                      {formatZuluFull(c.start_time)}
                      {c.end_time ? ` – ${formatZulu(c.end_time)}` : " – live"}
                    </td>
                    <td className="py-2 pr-3">
                      <Badge variant={c.status === "open" ? "success" : "secondary"}>
                        {c.status === "open" ? "recording" : "saved"}
                      </Badge>
                    </td>
                    <td className="py-2 text-right">
                      <Link
                        to="/historical/replay"
                        search={{ capture: c.id }}
                        className={buttonVariants({ variant: "outline", size: "sm" }) + " gap-1"}
                      >
                        <Film className="size-3.5" />
                        Replay
                      </Link>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
