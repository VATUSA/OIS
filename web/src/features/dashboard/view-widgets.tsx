import {useMemo, useState} from "react";
import {cn, EmptyState, Input} from "@ois/ui";
import {Filter} from "lucide-react";

import {AircraftView, DemandView, LadderView, SummaryView, summaryGateName} from "@/pages/airport";
import {DeparturesView} from "@/pages/departures";
import {TaxiView} from "@/pages/taxi";
import {type Flow, useAirportFlow} from "@/lib/feed";

import type {LadderFilters, ViewId, ViewWidget} from "./types";

/** Views that need only an ICAO and self-fetch. */
const AIRPORT_VIEWS = {
  "airport-summary": SummaryView,
  "airport-aircraft": AircraftView,
  "airport-demand": DemandView,
} as const;

const LADDER_STATUSES = ["airborne", "ground", "proposed"] as const;

function Notice({ children }: { children: React.ReactNode }) {
  return (
    <EmptyState>{children}</EmptyState>
  );
}

/** The airport views take a pre-fetched `flow`; fetch it here from the widget's ICAO. */
function AirportFlowView({ icao, view }: { icao: string; view: keyof typeof AIRPORT_VIEWS }) {
  const flow = useAirportFlow(icao);
  const View = AIRPORT_VIEWS[view];
  if (flow.isError) return <Notice>Couldn&apos;t load {icao}.</Notice>;
  if (!flow.data) return <Notice>Loading {icao}…</Notice>;
  return <View flow={flow.data} />;
}

/** A toggle chip (multi-select, so chips rather than a SegmentedControl). */
function chipClass(active: boolean): string {
  return cn(
    "rounded-full border px-2.5 py-0.5 font-mono transition-colors",
    active ? "border-brand/40 bg-brand-soft text-ink" : "border-line bg-panel-2 text-ink-2 hover:text-ink",
  );
}

const parseList = (raw: string): string[] =>
  raw
    .split(/[,\s]+/)
    .map((s) => s.replace(/[^a-zA-Z0-9]/g, "").toUpperCase())
    .filter(Boolean);

/** Edit-mode filter panel for the arrival-ladder widget. Persists via `onChange`. */
function LadderFilterBar({
  flow,
  filters,
  onChange,
}: {
  flow: Flow;
  filters: LadderFilters;
  onChange: (next: LadderFilters) => void;
}) {
  const [open, setOpen] = useState(false);
  const gates = useMemo(
    () =>
      [...new Set(flow.flights.map((fl) => summaryGateName(fl.gate)).filter((g): g is string => !!g))].sort(),
    [flow.flights],
  );
  const activeCount =
    (filters.gates?.length ? 1 : 0) +
    (filters.statuses?.length ? 1 : 0) +
    (filters.origins?.length ? 1 : 0) +
    (filters.types?.length ? 1 : 0);

  const toggleIn = (key: "gates" | "statuses", val: string) => {
    const cur = filters[key] ?? [];
    onChange({ ...filters, [key]: cur.includes(val) ? cur.filter((x) => x !== val) : [...cur, val] });
  };

  return (
    <div className="rounded-sm border border-line bg-panel-2 text-xs">
      <div className="flex items-center gap-2 px-2 py-1.5">
        <button
          type="button"
          onClick={() => setOpen((o) => !o)}
          className="flex items-center gap-1.5 font-semibold text-ink"
        >
          <Filter className="size-3.5" />
          Filters
          {activeCount > 0 && (
            <span className="rounded-full bg-brand-soft px-1.5 font-mono text-brand-ink">{activeCount}</span>
          )}
        </button>
        {activeCount > 0 && (
          <button
            type="button"
            onClick={() => onChange({})}
            className="ml-auto text-ink-3 hover:text-ink"
          >
            Clear
          </button>
        )}
      </div>

      {open && (
        <div className="flex flex-col gap-2.5 border-t border-line p-2">
          <div>
            <div className="mb-1 font-semibold text-ink-2">Status</div>
            <div className="flex flex-wrap gap-1">
              {LADDER_STATUSES.map((s) => (
                <button
                  key={s}
                  type="button"
                  className={chipClass((filters.statuses ?? []).includes(s))}
                  onClick={() => toggleIn("statuses", s)}
                >
                  {s}
                </button>
              ))}
            </div>
          </div>

          <div>
            <div className="mb-1 font-semibold text-ink-2">Arrival gate</div>
            {gates.length === 0 ? (
              <span className="text-ink-3">none in current data</span>
            ) : (
              <div className="flex flex-wrap gap-1">
                {gates.map((g) => (
                  <button
                    key={g}
                    type="button"
                    className={chipClass((filters.gates ?? []).includes(g))}
                    onClick={() => toggleIn("gates", g)}
                  >
                    {g}
                  </button>
                ))}
              </div>
            )}
          </div>

          <div className="flex gap-2">
            <label className="flex-1">
              <div className="mb-1 font-semibold text-ink-2">Origin (ICAO)</div>
              <Input
                key={`o-${(filters.origins ?? []).join(",")}`}
                defaultValue={(filters.origins ?? []).join(", ")}
                onBlur={(e) => onChange({ ...filters, origins: parseList(e.target.value) })}
                placeholder="KBOS, KIAD"
                className="h-7 font-mono text-xs"
              />
            </label>
            <label className="flex-1">
              <div className="mb-1 font-semibold text-ink-2">Aircraft type</div>
              <Input
                key={`t-${(filters.types ?? []).join(",")}`}
                defaultValue={(filters.types ?? []).join(", ")}
                onBlur={(e) => onChange({ ...filters, types: parseList(e.target.value) })}
                placeholder="B73, A32 (jets)"
                className="h-7 font-mono text-xs"
              />
            </label>
          </div>
        </div>
      )}
    </div>
  );
}

/** The arrival-ladder widget: fetches flow, shows the filter panel in edit mode, applies filters. */
function LadderWidget({
  widget,
  editing,
  onChange,
}: {
  widget: ViewWidget;
  editing: boolean;
  onChange: (id: string, patch: Record<string, unknown>) => void;
}) {
  const flow = useAirportFlow(widget.icao);
  if (flow.isError) return <Notice>Couldn&apos;t load {widget.icao}.</Notice>;
  if (!flow.data) return <Notice>Loading {widget.icao}…</Notice>;
  const filters = widget.filters ?? {};
  return (
    <div className="flex flex-col gap-2">
      {editing && (
        <LadderFilterBar
          flow={flow.data}
          filters={filters}
          onChange={(next) => onChange(widget.id, { filters: next })}
        />
      )}
      <LadderView flow={flow.data} filters={filters} />
    </div>
  );
}

/** Views available as widgets, and their labels for the add-widget menu. */
export const VIEW_OPTIONS: { id: ViewId; label: string }[] = [
  { id: "airport-summary", label: "Airport — Summary" },
  { id: "airport-aircraft", label: "Airport — Aircraft list" },
  { id: "airport-ladder", label: "Airport — Arrival ladder" },
  { id: "airport-demand", label: "Airport — Demand vs AAR" },
  { id: "departures", label: "Departures" },
  { id: "taxi", label: "Taxi times" },
];

export function ViewWidgetView({
  widget,
  editing,
  onChange,
}: {
  widget: ViewWidget;
  editing: boolean;
  onChange: (id: string, patch: Record<string, unknown>) => void;
}) {
  switch (widget.view) {
    case "departures":
      return <DeparturesView icao={widget.icao} />;
    case "taxi":
      return <TaxiView icao={widget.icao} />;
    case "airport-ladder":
      return <LadderWidget widget={widget} editing={editing} onChange={onChange} />;
    default:
      return <AirportFlowView icao={widget.icao} view={widget.view} />;
  }
}
