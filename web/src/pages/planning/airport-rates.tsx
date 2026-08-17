import {useEffect, useMemo, useState} from "react";
import {Badge, Button, Card, CardContent, ConfirmButton, Input} from "@ois/ui";
import {CloudSun, Gauge, Plus, Wand2, X} from "lucide-react";

import {useMe} from "@/lib/auth";
import {type AirportRate, useAirportRates, useRemoveAirportRate, useUpsertAirportRate,} from "@/lib/events";
import {type AirportConfig, matchConfig, useAirportConfigs, useForecast,} from "@/lib/airport-configs";
import {hasPermission} from "@/lib/permissions";

const clampRate = (n: number) => Math.max(0, Math.min(200, Math.round(n)));

/** "302° @ 8kt" (gust when notable) for a forecast wind, or a dash when unknown. */
function windLabel(dir: number | null | undefined, kt: number, gust?: number | null): string {
  if (dir == null) return kt > 0 ? `calm–var @ ${kt}kt` : "calm";
  const g = gust && gust >= kt + 5 ? ` G${gust}` : "";
  return `${String(dir).padStart(3, "0")}° @ ${kt}${g}kt`;
}

function RateRow({
  eventId,
  row,
  atUnix,
}: {
  eventId: number;
  row: AirportRate;
  atUnix: number | null;
}) {
  const upsert = useUpsertAirportRate(eventId);
  const remove = useRemoveAirportRate(eventId);
  const configs = useAirportConfigs(row.icao);
  const forecast = useForecast(row.icao, atUnix);
  const [aar, setAar] = useState(String(row.aar));
  const [adr, setAdr] = useState(String(row.adr));

  useEffect(() => {
    setAar(String(row.aar));
    setAdr(String(row.adr));
  }, [row.aar, row.adr]);

  const configList = useMemo(() => configs.data ?? [], [configs.data]);
  const windDir = forecast.data?.wind_dir ?? null;
  const predicted = useMemo(() => matchConfig(configList, windDir), [configList, windDir]);

  const forecastCell = forecast.data ? (
    forecast.data.source === "none" ? (
      <span className="text-muted-foreground">no forecast</span>
    ) : (
      <span className="font-mono text-xs">
        {windLabel(forecast.data.wind_dir, forecast.data.wind_kt, forecast.data.gust_kt)}
      </span>
    )
  ) : (
    <span className="text-muted-foreground">…</span>
  );

  const IcaoCell = (
    <td className="py-2 pr-3 align-top">
      <span className="font-mono font-medium">{row.icao}</span>
      {row.artcc && <span className="ml-2 text-xs text-muted-foreground">{row.artcc}</span>}
    </td>
  );

  // Read-only view for airports outside the caller's facility scope.
  if (!row.editable) {
    return (
      <tr className="border-t">
        {IcaoCell}
        <td className="py-2 pr-3 align-top">{forecastCell}</td>
        <td className="py-2 pr-3 align-top text-xs text-muted-foreground">
          {row.config_id ? configList.find((c) => c.id === row.config_id)?.name ?? "—" : "manual"}
        </td>
        <td className="py-2 pr-3 align-top tabular-nums">
          {row.aar} / {row.adr}
        </td>
        <td className="py-2 pr-3 align-top">
          <SourceBadge source={row.source} />
        </td>
        <td className="py-2" />
      </tr>
    );
  }

  const applyConfig = (c: AirportConfig, source: "predicted" | "override") =>
    upsert.mutate({
      icao: row.icao,
      body: { aar: c.aar, adr: c.adr, config_id: c.id, source },
    });

  const onPickConfig = (value: string) => {
    if (value === "manual") {
      upsert.mutate({
        icao: row.icao,
        body: {
          aar: clampRate(Number(aar) || 0),
          adr: clampRate(Number(adr) || 0),
          config_id: null,
          source: "override",
        },
      });
      return;
    }
    const c = configList.find((x) => x.id === value);
    if (c) applyConfig(c, predicted && c.id === predicted.id ? "predicted" : "override");
  };

  const saveManual = () => {
    const a = clampRate(Number(aar) || 0);
    const d = clampRate(Number(adr) || 0);
    if (a !== row.aar || d !== row.adr || row.config_id) {
      upsert.mutate({ icao: row.icao, body: { aar: a, adr: d, config_id: null, source: "override" } });
    }
  };

  const selectValue = row.config_id ?? "manual";
  const predictedMismatch = predicted && row.config_id !== predicted.id;

  return (
    <tr className="border-t align-top">
      {IcaoCell}
      <td className="py-2 pr-3">
        <div className="flex flex-col gap-1">
          {forecastCell}
          {predicted && (
            <span className="flex items-center gap-1 text-xs text-muted-foreground">
              → {predicted.name} ({predicted.aar}/{predicted.adr})
              {predictedMismatch && (
                <button
                  type="button"
                  className="inline-flex items-center gap-0.5 rounded px-1 text-primary hover:underline"
                  onClick={() => applyConfig(predicted, "predicted")}
                  title="Apply the predicted config"
                >
                  <Wand2 className="size-3" />
                  use
                </button>
              )}
            </span>
          )}
        </div>
      </td>
      <td className="py-2 pr-3">
        <select
          className="h-8 rounded-md border bg-background px-2 text-sm"
          value={selectValue}
          onChange={(e) => onPickConfig(e.target.value)}
        >
          {configList.map((c) => (
            <option key={c.id} value={c.id}>
              {c.name}
              {predicted && c.id === predicted.id ? " (predicted)" : ""}
            </option>
          ))}
          <option value="manual">Manual</option>
        </select>
      </td>
      <td className="py-2 pr-3">
        <div className="flex items-center gap-1">
          <Input
            className="h-8 w-16 tabular-nums"
            type="number"
            min={0}
            max={200}
            value={aar}
            onChange={(e) => setAar(e.target.value)}
            onBlur={saveManual}
          />
          <span className="text-muted-foreground">/</span>
          <Input
            className="h-8 w-16 tabular-nums"
            type="number"
            min={0}
            max={200}
            value={adr}
            onChange={(e) => setAdr(e.target.value)}
            onBlur={saveManual}
          />
        </div>
      </td>
      <td className="py-2 pr-3">
        <SourceBadge source={row.source} />
      </td>
      <td className="py-2 text-right">
        <ConfirmButton
          size="icon"
          title={`Remove ${row.icao}`}
          aria-label={`Remove ${row.icao}`}
          onConfirm={() => remove.mutate(row.icao)}
          warn={`Remove the ${row.icao} rate?`}
        >
          <X className="size-4" />
        </ConfirmButton>
      </td>
    </tr>
  );
}

function SourceBadge({ source }: { source?: string | null }) {
  if (source === "predicted")
    return (
      <Badge variant="secondary" className="gap-1">
        <CloudSun className="size-3" />
        predicted
      </Badge>
    );
  return <Badge variant="outline">override</Badge>;
}

export function AirportRatesSection({
  eventId,
  eventStart,
}: {
  eventId: number;
  eventStart: string;
}) {
  const { data: me } = useMe();
  const canEdit = hasPermission(me, "events.rate.update");
  const rates = useAirportRates(eventId);
  const upsert = useUpsertAirportRate(eventId);
  const [query, setQuery] = useState("");

  const atUnix = useMemo(() => {
    const ms = Date.parse(eventStart);
    return Number.isFinite(ms) ? Math.floor(ms / 1000) : null;
  }, [eventStart]);

  const rows = rates.data ?? [];

  function add() {
    const icao = query.replace(/[^a-zA-Z0-9]/g, "").toUpperCase();
    if (icao.length >= 3 && icao.length <= 4 && !rows.some((r) => r.icao === icao)) {
      upsert.mutate({ icao, body: { aar: 30, adr: 30, source: "override" } });
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
            <span className="font-semibold">Airports &amp; rates</span>
            <span className="text-xs text-muted-foreground">
              Forecast wind at event start picks a config; override any airport. Facility staff edit
              only their own airports.
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
          <p className="py-2 text-sm text-muted-foreground">No airport rates set yet.</p>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-xs uppercase tracking-wide text-muted-foreground">
                  <th className="pb-2 pr-3 font-medium">Airport</th>
                  <th className="pb-2 pr-3 font-medium">Forecast</th>
                  <th className="pb-2 pr-3 font-medium">Config</th>
                  <th className="pb-2 pr-3 font-medium">AAR / ADR</th>
                  <th className="pb-2 pr-3 font-medium">Source</th>
                  <th className="pb-2" />
                </tr>
              </thead>
              <tbody>
                {rows.map((row) => (
                  <RateRow key={row.icao} eventId={eventId} row={row} atUnix={atUnix} />
                ))}
              </tbody>
            </table>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
