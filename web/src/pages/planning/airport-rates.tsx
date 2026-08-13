import {useEffect, useState} from "react";
import {Button, Card, CardContent, Input} from "@ois/ui";
import {Gauge, Plus, X} from "lucide-react";

import {useMe} from "@/lib/auth";
import {type AirportRate, useAirportRates, useRemoveAirportRate, useUpsertAirportRate,} from "@/lib/events";
import {hasPermission} from "@/lib/permissions";

const clampRate = (n: number) => Math.max(0, Math.min(200, Math.round(n)));

function RateRow({ eventId, row }: { eventId: number; row: AirportRate }) {
  const upsert = useUpsertAirportRate(eventId);
  const remove = useRemoveAirportRate(eventId);
  const [aar, setAar] = useState(String(row.aar));
  const [adr, setAdr] = useState(String(row.adr));

  useEffect(() => {
    setAar(String(row.aar));
    setAdr(String(row.adr));
  }, [row.aar, row.adr]);

  const IcaoCell = (
    <td className="py-2 pr-3">
      <span className="font-mono font-medium">{row.icao}</span>
      {row.artcc && (
        <span className="ml-2 text-xs text-muted-foreground">{row.artcc}</span>
      )}
    </td>
  );

  if (!row.editable) {
    return (
      <tr className="border-t">
        {IcaoCell}
        <td className="py-2 pr-3 tabular-nums">{row.aar}</td>
        <td className="py-2 pr-3 tabular-nums">{row.adr}</td>
        <td className="py-2 text-right text-xs text-muted-foreground">
          {row.updated_by ?? ""}
        </td>
      </tr>
    );
  }

  const save = () => {
    const a = clampRate(Number(aar) || 0);
    const d = clampRate(Number(adr) || 0);
    if (a !== row.aar || d !== row.adr) {
      upsert.mutate({ icao: row.icao, body: { aar: a, adr: d } });
    }
  };

  return (
    <tr className="border-t">
      {IcaoCell}
      <td className="py-2 pr-3">
        <Input
          className="h-8 w-16 tabular-nums"
          type="number"
          min={0}
          max={200}
          value={aar}
          onChange={(e) => setAar(e.target.value)}
          onBlur={save}
        />
      </td>
      <td className="py-2 pr-3">
        <Input
          className="h-8 w-16 tabular-nums"
          type="number"
          min={0}
          max={200}
          value={adr}
          onChange={(e) => setAdr(e.target.value)}
          onBlur={save}
        />
      </td>
      <td className="py-2 text-right">
        <button
          type="button"
          title={`Remove ${row.icao}`}
          onClick={() => remove.mutate(row.icao)}
          className="text-muted-foreground transition-colors hover:text-destructive"
        >
          <X className="size-4" />
        </button>
      </td>
    </tr>
  );
}

export function AirportRatesSection({ eventId }: { eventId: number }) {
  const { data: me } = useMe();
  const canEdit = hasPermission(me, "events.rate.update");
  const rates = useAirportRates(eventId);
  const upsert = useUpsertAirportRate(eventId);
  const [query, setQuery] = useState("");

  const rows = rates.data ?? [];

  function add() {
    const icao = query.replace(/[^a-zA-Z0-9]/g, "").toUpperCase();
    if (icao.length >= 3 && icao.length <= 4 && !rows.some((r) => r.icao === icao)) {
      upsert.mutate({ icao, body: { aar: 30, adr: 30 } });
    }
    setQuery("");
  }

  return (
    <Card>
      <CardContent className="flex flex-col gap-4 pt-6">
        <div className="flex items-center gap-2">
          <span className="flex size-8 items-center justify-center rounded-md bg-primary/10 text-primary">
            <Gauge className="size-4" />
          </span>
          <div className="flex flex-col">
            <span className="font-semibold">Airport rates</span>
            <span className="text-xs text-muted-foreground">
              Planned AAR / ADR per airport. Facility staff edit only their own
              airports.
            </span>
          </div>
        </div>

        {canEdit && (
          <div className="flex flex-wrap items-center gap-2">
            <Input
              className="w-28 font-mono uppercase"
              maxLength={4}
              placeholder="KATL"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && add()}
            />
            <Button onClick={add}>
              <Plus />
              Add airport
            </Button>
          </div>
        )}

        {!rates.data ? (
          <p className="py-2 text-sm text-muted-foreground">Loading…</p>
        ) : rows.length === 0 ? (
          <p className="py-2 text-sm text-muted-foreground">
            No airport rates set yet.
          </p>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-xs uppercase tracking-wide text-muted-foreground">
                  <th className="pb-2 pr-3 font-medium">Airport</th>
                  <th className="pb-2 pr-3 font-medium">AAR</th>
                  <th className="pb-2 pr-3 font-medium">ADR</th>
                  <th className="pb-2" />
                </tr>
              </thead>
              <tbody>
                {rows.map((row) => (
                  <RateRow key={row.icao} eventId={eventId} row={row} />
                ))}
              </tbody>
            </table>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
