import {useEffect, useMemo, useState} from "react";
import {Button, ConfirmButton, type DataColumn, DataTable, FilterBar, Input, Select, StatusPill} from "@ois/ui";
import {CloudSun, Gauge, Plane, Plus, Settings2, Tag, Wand2, Wind, X} from "lucide-react";

import {useMe} from "@/lib/auth";
import {type AirportRate, useAirportRates, useRemoveAirportRate, useUpsertAirportRate} from "@/lib/events";
import {type AirportConfig, matchConfig, useAirportConfigs, useForecast} from "@/lib/airport-configs";
import {hasPermission} from "@/lib/permissions";
import {SectionHeader} from "@/pages/planning/section-header";

const clampRate = (n: number) => Math.max(0, Math.min(200, Math.round(n)));

/** "302° @ 8kt" (gust when notable) for a forecast wind, or a dash when unknown. */
function windLabel(dir: number | null | undefined, kt: number, gust?: number | null): string {
  if (dir == null) return kt > 0 ? `calm–var @ ${kt}kt` : "calm";
  const g = gust && gust >= kt + 5 ? ` G${gust}` : "";
  return `${String(dir).padStart(3, "0")}° @ ${kt}${g}kt`;
}

/** The airport's configs, its forecast at event start, and the config that forecast predicts. */
function usePrediction(icao: string, atUnix: number | null) {
  const configs = useAirportConfigs(icao);
  const forecast = useForecast(icao, atUnix);
  const configList = useMemo(() => configs.data ?? [], [configs.data]);
  const windDir = forecast.data?.wind_dir ?? null;
  const predicted = useMemo(() => matchConfig(configList, windDir), [configList, windDir]);
  return { configList, forecast, predicted };
}

function useApplyConfig(eventId: number, icao: string) {
  const upsert = useUpsertAirportRate(eventId);
  return (c: AirportConfig, source: "predicted" | "override") =>
    upsert.mutate({ icao, body: { aar: c.aar, adr: c.adr, config_id: c.id, source } });
}

function ForecastCell({ eventId, row, atUnix }: { eventId: number; row: AirportRate; atUnix: number | null }) {
  const { forecast, predicted } = usePrediction(row.icao, atUnix);
  const applyConfig = useApplyConfig(eventId, row.icao);

  const wind = forecast.data ? (
    forecast.data.source === "none" ? (
      <span className="text-ink-3">no forecast</span>
    ) : (
      <span className="font-mono text-xs">
        {windLabel(forecast.data.wind_dir, forecast.data.wind_kt, forecast.data.gust_kt)}
      </span>
    )
  ) : (
    <span className="text-ink-3">…</span>
  );

  if (!row.editable) return wind;

  return (
    <div className="flex flex-col gap-1">
      {wind}
      {predicted && (
        <span className="flex items-center gap-1 text-xs text-ink-2">
          → {predicted.name} <span className="font-mono">({predicted.aar}/{predicted.adr})</span>
          {row.config_id !== predicted.id && (
            <button
              type="button"
              className="inline-flex items-center gap-0.5 rounded-xs px-1 font-semibold text-brand-ink hover:underline"
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
  );
}

function ConfigCell({ eventId, row, atUnix }: { eventId: number; row: AirportRate; atUnix: number | null }) {
  const { configList, predicted } = usePrediction(row.icao, atUnix);
  const upsert = useUpsertAirportRate(eventId);
  const applyConfig = useApplyConfig(eventId, row.icao);

  if (!row.editable) {
    return (
      <span className="text-xs text-ink-2">
        {row.config_id ? (configList.find((c) => c.id === row.config_id)?.name ?? "—") : "manual"}
      </span>
    );
  }

  const onPickConfig = (value: string) => {
    if (value === "manual") {
      upsert.mutate({
        icao: row.icao,
        body: { aar: row.aar, adr: row.adr, config_id: null, source: "override" },
      });
      return;
    }
    const c = configList.find((x) => x.id === value);
    if (c) applyConfig(c, predicted && c.id === predicted.id ? "predicted" : "override");
  };

  return (
    <Select size="sm" value={row.config_id ?? "manual"} onChange={(e) => onPickConfig(e.target.value)}>
      {configList.map((c) => (
        <option key={c.id} value={c.id}>
          {c.name}
          {predicted && c.id === predicted.id ? " (predicted)" : ""}
        </option>
      ))}
      <option value="manual">Manual</option>
    </Select>
  );
}

function RatesCell({ eventId, row }: { eventId: number; row: AirportRate }) {
  const upsert = useUpsertAirportRate(eventId);
  const [aar, setAar] = useState(String(row.aar));
  const [adr, setAdr] = useState(String(row.adr));

  useEffect(() => {
    setAar(String(row.aar));
    setAdr(String(row.adr));
  }, [row.aar, row.adr]);

  if (!row.editable) {
    return (
      <span className="font-mono">
        {row.aar} / {row.adr}
      </span>
    );
  }

  const saveManual = () => {
    const a = clampRate(Number(aar) || 0);
    const d = clampRate(Number(adr) || 0);
    if (a !== row.aar || d !== row.adr || row.config_id) {
      upsert.mutate({ icao: row.icao, body: { aar: a, adr: d, config_id: null, source: "override" } });
    }
  };

  return (
    <div className="flex items-center gap-1">
      <Input
        aria-label={`${row.icao} AAR`}
        className="h-8 w-16 font-mono"
        type="number"
        min={0}
        max={200}
        value={aar}
        onChange={(e) => setAar(e.target.value)}
        onBlur={saveManual}
      />
      <span className="text-ink-3">/</span>
      <Input
        aria-label={`${row.icao} ADR`}
        className="h-8 w-16 font-mono"
        type="number"
        min={0}
        max={200}
        value={adr}
        onChange={(e) => setAdr(e.target.value)}
        onBlur={saveManual}
      />
    </div>
  );
}

function SourcePill({ source }: { source?: string | null }) {
  if (source === "predicted")
    return (
      <StatusPill tone="brand">
        <CloudSun className="size-3" />
        predicted
      </StatusPill>
    );
  return <StatusPill tone="neutral">override</StatusPill>;
}

function RemoveCell({ eventId, row }: { eventId: number; row: AirportRate }) {
  const remove = useRemoveAirportRate(eventId);
  if (!row.editable) return null;
  return (
    <ConfirmButton
      size="icon"
      title={`Remove ${row.icao}`}
      aria-label={`Remove ${row.icao}`}
      onConfirm={() => remove.mutate(row.icao)}
      warn={`Remove the ${row.icao} rate?`}
    >
      <X className="size-4" />
    </ConfirmButton>
  );
}

export function AirportRatesSection({ eventId, eventStart }: { eventId: number; eventStart: string }) {
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

  const columns = useMemo<DataColumn<AirportRate>[]>(
    () => [
      {
        accessorKey: "icao",
        header: "Airport",
        icon: Plane,
        cell: (c) => (
          <span className="whitespace-nowrap">
            <span className="font-mono font-semibold">{c.row.original.icao}</span>
            {c.row.original.artcc && (
              <span className="ml-2 font-mono text-xs text-ink-3">{c.row.original.artcc}</span>
            )}
          </span>
        ),
      },
      {
        id: "forecast",
        header: "Forecast",
        icon: Wind,
        enableSorting: false,
        cell: (c) => <ForecastCell eventId={eventId} row={c.row.original} atUnix={atUnix} />,
      },
      {
        id: "config",
        header: "Config",
        icon: Settings2,
        enableSorting: false,
        cell: (c) => <ConfigCell eventId={eventId} row={c.row.original} atUnix={atUnix} />,
      },
      {
        accessorKey: "aar",
        header: "AAR / ADR",
        icon: Gauge,
        cell: (c) => <RatesCell eventId={eventId} row={c.row.original} />,
      },
      {
        accessorKey: "source",
        header: "Source",
        icon: Tag,
        cell: (c) => <SourcePill source={c.row.original.source} />,
      },
      {
        id: "actions",
        header: () => <span className="sr-only">Actions</span>,
        enableSorting: false,
        align: "right",
        cell: (c) => <RemoveCell eventId={eventId} row={c.row.original} />,
      },
    ],
    [eventId, atUnix],
  );

  function add() {
    const icao = query.replace(/[^a-zA-Z0-9]/g, "").toUpperCase();
    if (icao.length >= 3 && icao.length <= 4 && !rows.some((r) => r.icao === icao)) {
      upsert.mutate({ icao, body: { aar: 30, adr: 30, source: "override" } });
    }
    setQuery("");
  }

  return (
    <section className="flex flex-col gap-4">
      <SectionHeader
        title="Airports & rates"
        description="Forecast wind at event start picks a config; override any airport. Facility staff edit only their own airports."
      />

      {canEdit && (
        <FilterBar>
          <Input
            aria-label="Airport ICAO"
            className="w-28 font-mono uppercase"
            maxLength={4}
            placeholder="KATL"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && add()}
          />
          <Button size="sm" onClick={add}>
            <Plus />
            Add airport
          </Button>
        </FilterBar>
      )}

      <DataTable
        label="Airport rates"
        columns={columns}
        data={rows}
        getRowId={(r) => r.icao}
        rowCap={25}
        isLoading={rates.isLoading}
        isError={rates.isError}
        onRetry={() => rates.refetch()}
        empty="No airport rates set yet."
      />
    </section>
  );
}
