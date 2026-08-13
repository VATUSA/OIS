import {useEffect, useMemo, useState} from "react";
import {Button, Input} from "@ois/ui";
import {Plus, X} from "lucide-react";

import {type TaxiActive, useMultiTaxiStats} from "@/lib/taxi";

const STORE_KEY = "ois.taxi.fields";
const MAX_FIELDS = 5;

function useFields() {
  const [fields, setFields] = useState<string[]>(() => {
    try {
      const raw = localStorage.getItem(STORE_KEY);
      return raw ? (JSON.parse(raw) as string[]) : [];
    } catch {
      return [];
    }
  });
  useEffect(() => {
    localStorage.setItem(STORE_KEY, JSON.stringify(fields));
  }, [fields]);
  return [fields, setFields] as const;
}

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

export function TaxiMonitorPage() {
  const [fields, setFields] = useFields();
  const [query, setQuery] = useState("");
  const now = useNow();
  const results = useMultiTaxiStats(fields);

  function add() {
    const f = query.replace(/[^a-zA-Z0-9]/g, "").toUpperCase();
    if (f.length >= 3 && f.length <= 4 && !fields.includes(f) && fields.length < MAX_FIELDS) {
      setFields([...fields, f]);
    }
    setQuery("");
  }

  // Flatten every field's in-progress departures into one table, tag with the field.
  const rows: Row[] = fields.flatMap((icao, i) =>
    (results[i].data?.active ?? []).map((a) => ({ ...a, field: icao })),
  );

  const summary = useMemo(() => {
    const rolling = rows.filter((r) => r.phase === "rolling").length;
    const watching = rows.filter((r) => r.phase === "watching").length;
    const samples = fields.reduce(
      (n, _f, i) => n + (results[i].data?.sample_count ?? 0),
      0,
    );
    const avgs = fields
      .map((_f, i) => results[i].data?.avg_min)
      .filter((v): v is number => v != null);
    const avg =
      avgs.length > 0
        ? Math.round(avgs.reduce((a, b) => a + b, 0) / avgs.length)
        : null;
    return { rolling, watching, samples, avg };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rows.length, fields.join(","), JSON.stringify(results.map((r) => r.data?.sample_count))]);

  return (
    <div className="flex flex-col gap-6">
      {/* Add bar */}
      <div className="flex flex-wrap items-center gap-3">
        <label className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
          Add airport
        </label>
        <Input
          className="w-28 font-mono uppercase"
          maxLength={4}
          placeholder="ICAO"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && add()}
        />
        <Button onClick={add} disabled={fields.length >= MAX_FIELDS}>
          <Plus />
          Add
        </Button>
        <span className="text-xs text-muted-foreground">
          {fields.length} / {MAX_FIELDS} airports · timer: {">"}7 kt start, 60 kt
          or climb stop
        </span>
      </div>

      {/* Field chips */}
      {fields.length > 0 && (
        <div className="flex flex-wrap gap-2">
          {fields.map((icao) => (
            <span
              key={icao}
              className="flex items-center gap-2 rounded-md border bg-card px-3 py-1.5 font-mono text-sm"
            >
              {icao}
              <button
                type="button"
                title={`Remove ${icao}`}
                onClick={() => setFields(fields.filter((x) => x !== icao))}
                className="text-muted-foreground transition-colors hover:text-destructive"
              >
                <X className="size-3.5" />
              </button>
            </span>
          ))}
        </div>
      )}

      {/* Header + aggregate stats */}
      <div className="flex flex-wrap items-center gap-x-8 gap-y-3 border-b border-border/60 pb-4">
        <h1 className="text-2xl font-semibold tracking-tight">Taxi monitor</h1>
        <Stat label="Fields" value={String(fields.length)} />
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
      </div>

      {/* Live table */}
      {fields.length === 0 ? (
        <div className="rounded-lg border py-12 text-center text-sm text-muted-foreground">
          Add the departure fields you want to watch.
        </div>
      ) : rows.length === 0 ? (
        <div className="rounded-lg border py-12 text-center text-sm text-muted-foreground">
          No departures taxiing right now. Rows appear as aircraft push and roll.
        </div>
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
    </div>
  );
}
