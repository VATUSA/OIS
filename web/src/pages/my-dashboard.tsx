import {useEffect, useState} from "react";
import {Button, Card, CardContent, Input} from "@ois/ui";
import {Plus, X} from "lucide-react";

import {useMe} from "@/lib/auth";
import {type Departure, useMultiDepartures} from "@/lib/departures";
import {hasPermission} from "@/lib/permissions";
import {DepartureRow} from "@/pages/departures";

const STORE_KEY = "ois.mydash.fields";
const MAX_FIELDS = 12;

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

function cfrMs(d: Departure): number {
  return d.cfr ? new Date(d.cfr).getTime() : Number.MAX_SAFE_INTEGER;
}

export function MyDashboardPage() {
  const { data: me } = useMe();
  const canIssue = hasPermission(me, "tmu.cfr.assign");
  const [fields, setFields] = useFields();
  const [query, setQuery] = useState("");
  const results = useMultiDepartures(fields);

  function addField() {
    const f = query.replace(/[^a-zA-Z0-9]/g, "").toUpperCase();
    if (f.length >= 3 && f.length <= 4 && !fields.includes(f) && fields.length < MAX_FIELDS) {
      setFields([...fields, f]);
    }
    setQuery("");
  }

  // Combine every field's departures into one list, tagged with its origin field.
  const rows = fields
    .flatMap((f, i) =>
      (results[i].data?.departures ?? []).map((d) => ({ from: f, d })),
    )
    .sort((a, b) => cfrMs(a.d) - cfrMs(b.d));

  const loading = results.some((r) => r.isLoading);
  const total = rows.length;
  const holding = fields.reduce(
    (n, _f, i) => n + (results[i].data?.holding_on_cfr ?? 0),
    0,
  );

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">My dashboard</h1>
        <p className="text-muted-foreground">
          Your fields, combined. Pending departures across everything you&apos;re
          working, with their Call-For-Release times. Field list stays on this device.
        </p>
      </div>

      <Card>
        <CardContent className="flex flex-col gap-4 pt-6">
          <div className="flex flex-wrap items-end gap-3">
            <label className="flex flex-col gap-1 text-xs font-medium uppercase tracking-wide text-muted-foreground">
              Add field
              <Input
                className="w-32 font-mono uppercase"
                maxLength={4}
                placeholder="KBOS"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && addField()}
              />
            </label>
            <Button onClick={addField} disabled={fields.length >= MAX_FIELDS}>
              <Plus />
              Add
            </Button>
            {fields.length > 0 && (
              <span className="ml-auto text-xs text-muted-foreground">
                {loading ? "refreshing…" : "live · updates every 20s"}
              </span>
            )}
          </div>
          {fields.length > 0 && (
            <div className="flex flex-wrap gap-2">
              {fields.map((f) => (
                <span
                  key={f}
                  className="flex items-center gap-1.5 rounded-full border py-1 pl-3 pr-1.5 font-mono text-sm"
                >
                  {f}
                  <button
                    type="button"
                    title={`Remove ${f}`}
                    onClick={() => setFields(fields.filter((x) => x !== f))}
                    className="flex size-5 items-center justify-center rounded-full text-muted-foreground transition-colors hover:text-destructive"
                  >
                    <X className="size-3.5" />
                  </button>
                </span>
              ))}
            </div>
          )}
        </CardContent>
      </Card>

      {fields.length === 0 ? (
        <Card>
          <CardContent className="py-12 text-center text-sm text-muted-foreground">
            Add the fields you&apos;re working to see all your departures in one place.
          </CardContent>
        </Card>
      ) : (
        <>
          <Card>
            <CardContent className="flex flex-wrap gap-10 pt-6">
              <div className="flex flex-col">
                <span className="text-3xl font-semibold tabular-nums">{total}</span>
                <span className="text-xs uppercase tracking-wide text-muted-foreground">
                  Pending departures
                </span>
              </div>
              <div className="flex flex-col">
                <span className="text-3xl font-semibold tabular-nums">{holding}</span>
                <span className="text-xs uppercase tracking-wide text-muted-foreground">
                  Holding on CFR
                </span>
              </div>
            </CardContent>
          </Card>

          <Card>
            <CardContent className="pt-6">
              {rows.length === 0 ? (
                <p className="py-8 text-center text-sm text-muted-foreground">
                  {loading ? "Loading…" : "No pending departures out of your fields."}
                </p>
              ) : (
                <div className="overflow-x-auto">
                  <table className="w-full text-sm">
                    <thead>
                      <tr className="text-left text-xs uppercase tracking-wide text-muted-foreground">
                        <th className="pb-2 pr-3 font-medium">From</th>
                        <th className="pb-2 pr-3 font-medium">Callsign</th>
                        <th className="pb-2 pr-3 font-medium">To</th>
                        <th className="pb-2 pr-3 font-medium">Type</th>
                        <th className="pb-2 pr-3 font-medium">Gate</th>
                        <th className="pb-2 pr-3 font-medium">ETA</th>
                        <th className="pb-2 pr-3 text-right font-medium">Delay</th>
                        <th className="pb-2 pr-3 text-right font-medium">CFR</th>
                        <th className="pb-2 pr-3 font-medium">Ready</th>
                        <th className="pb-2 pr-3 font-medium" />
                        <th className="pb-2" />
                      </tr>
                    </thead>
                    <tbody>
                      {rows.map(({ from, d }) => (
                        <DepartureRow
                          key={`${from}-${d.callsign}`}
                          d={d}
                          canIssue={canIssue}
                          leading={<span className="font-mono text-xs">{from}</span>}
                        />
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </CardContent>
          </Card>
        </>
      )}
    </div>
  );
}
