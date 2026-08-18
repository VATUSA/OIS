import {useEffect, useState} from "react";
import {Card, CardContent} from "@ois/ui";

import {useHistoricalAt} from "@/lib/historical-context";
import {type TaxiActive, useTaxiStats} from "@/lib/taxi";

/** A clock that ticks every second, so rolling timers advance between polls. */
function useNow() {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, []);
  return now;
}

function fmtClock(secs: number) {
  const s = Math.max(0, Math.floor(secs));
  const m = Math.floor(s / 60);
  return `${m}:${String(s % 60).padStart(2, "0")}`;
}

type Row = TaxiActive & { field: string };

function Stat({ label, value, tone }: { label: string; value: string; tone?: string }) {
  return (
    <div className="flex items-baseline gap-2">
      <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
        {label}
      </span>
      <span className={"text-lg font-semibold tabular-nums " + (tone ?? "")}>
        {value}
      </span>
    </div>
  );
}

function TaxiRow({ row, now }: { row: Row; now: number }) {
  const rolling = row.phase === "rolling";
  const elapsed =
    rolling && row.rolling_since
      ? (now - new Date(row.rolling_since).getTime()) / 1000
      : null;

  return (
    <tr className="border-t border-border/60">
      <td className="py-2 pr-4 font-mono font-semibold">{row.field}</td>
      <td className="py-2 pr-4 font-mono font-semibold">{row.callsign}</td>
      <td className="py-2 pr-4 font-mono text-muted-foreground">
        {row.dest || "—"}
      </td>
      <td className="py-2 pr-4 text-right tabular-nums">{row.gs}</td>
      <td className="py-2 pr-4 text-right tabular-nums">{row.alt}</td>
      <td className="py-2 pr-4">
        <span
          className={
            "font-mono text-xs uppercase tracking-wide " +
            (rolling ? "text-amber-500" : "text-sky-400")
          }
        >
          {row.phase}
        </span>
      </td>
      <td className="py-2 text-right font-mono tabular-nums">
        {elapsed != null ? (
          <span className="text-amber-500">{fmtClock(elapsed)}</span>
        ) : (
          <span className="text-muted-foreground">—</span>
        )}
      </td>
    </tr>
  );
}

/**
 * Live taxi-out picture for a single airport — a tab on the Airport page. From the start of the
 * roll ({">"}7 kt) to wheels-up (60 kt or a climb).
 */
export function TaxiView({ icao }: { icao: string }) {
  const now = useNow();
  const stats = useTaxiStats(icao);
  // Taxi timing is a rolling in-memory state machine, not reconstructable from a past snapshot.
  const historical = useHistoricalAt() != null;
  if (historical) {
    return (
      <Card>
        <CardContent className="py-8 text-center text-sm text-muted-foreground">
          Taxi timing isn&apos;t available in historical replay.
        </CardContent>
      </Card>
    );
  }

  const rows: Row[] = (stats.data?.active ?? []).map((a) => ({ ...a, field: icao }));
  const summary = {
    rolling: rows.filter((r) => r.phase === "rolling").length,
    watching: rows.filter((r) => r.phase === "watching").length,
    samples: stats.data?.sample_count ?? 0,
    avg: stats.data?.avg_min ?? null,
  };

  return (
    <Card>
      <CardContent className="flex flex-col gap-4 pt-6">
        <div className="flex flex-wrap items-center gap-x-8 gap-y-3 border-b border-border/60 pb-4">
          <Stat
            label="Rolling"
            value={String(summary.rolling)}
            tone={summary.rolling > 0 ? "text-amber-500" : ""}
          />
          <Stat
            label="Watching"
            value={String(summary.watching)}
            tone={summary.watching > 0 ? "text-sky-400" : ""}
          />
          <Stat
            label="Avg today"
            value={summary.avg != null ? `${summary.avg}m` : "—"}
          />
          <Stat label="Samples" value={String(summary.samples)} />
          <span className="ml-auto text-xs text-muted-foreground">
            live · updates every 15s
          </span>
        </div>

        {rows.length === 0 ? (
          <p className="py-8 text-center text-sm text-muted-foreground">
            No departures taxiing out of {icao} right now. Rows appear as aircraft push and
            roll.
          </p>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full min-w-[640px] text-sm">
              <thead>
                <tr className="text-left text-xs uppercase tracking-wide text-muted-foreground">
                  <th className="pb-2 pr-4 font-medium">Field</th>
                  <th className="pb-2 pr-4 font-medium">Callsign</th>
                  <th className="pb-2 pr-4 font-medium">Dest</th>
                  <th className="pb-2 pr-4 text-right font-medium">GS</th>
                  <th className="pb-2 pr-4 text-right font-medium">Alt</th>
                  <th className="pb-2 pr-4 font-medium">Phase</th>
                  <th className="pb-2 text-right font-medium">Taxi time</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((row) => (
                  <TaxiRow key={`${row.field}-${row.callsign}`} row={row} now={now} />
                ))}
              </tbody>
            </table>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
