// The data-source registry: each source wraps an existing React Query hook and exposes a flat
// `rows[]` plus a typed `fields[]` descriptor. Table (and later chart) widgets consume this, so
// adding a source here lights it up everywhere. Every `useRows` is a real hook — a widget binds
// one source for its lifetime, so calling `source.useRows(...)` unconditionally is hooks-safe
// (the TableWidget also keys its inner component by source id, remounting on any change).

import {useMultiDepartures} from "@/lib/departures";
import {useFcas, useTraffic} from "@/lib/fca";
import {useMultiAirportFlow} from "@/lib/feed";
import {usePrograms, useTmis} from "@/lib/tmu";
import {useMultiTaxiStats} from "@/lib/taxi";

/** Row field that tags which airport a row came from, for multi-airport comparison charts. */
export const AIRPORT_KEY = "__airport";

export type FieldType = "string" | "number" | "time" | "bool";

export interface FieldDef {
  key: string;
  label: string;
  type: FieldType;
}

export type Row = Record<string, unknown>;

export interface RowsResult {
  rows: Row[];
  isLoading: boolean;
  isError: boolean;
}

export interface SourceParams {
  icao?: string;
  /** One or more airports; airport sources tag each row with AIRPORT_KEY for comparison. */
  icaos?: string[];
}

export interface DataSource {
  id: string;
  label: string;
  category: "airport" | "global";
  /** True → the widget must be bound to at least one airport. */
  needsIcao: boolean;
  fields: FieldDef[];
  useRows: (params: SourceParams) => RowsResult;
}

const f = (key: string, label: string, type: FieldType = "string"): FieldDef => ({ key, label, type });

/** Normalize params to a concrete airport list (icaos preferred, else the single icao). */
const airports = (p: SourceParams): string[] => (p.icaos?.length ? p.icaos : p.icao ? [p.icao] : []);

export const DATA_SOURCES: DataSource[] = [
  {
    id: "airport-flow",
    label: "Airport arrivals",
    category: "airport",
    needsIcao: true,
    fields: [
      f("callsign", "Callsign"),
      f("dep", "From"),
      f("status", "Status"),
      f("seq", "Seq", "number"),
      f("eta", "ETA", "time"),
      f("sta", "STA", "time"),
      f("delay_min", "Delay", "number"),
      f("cfr", "CFR", "time"),
      f("gate", "Gate"),
      f("groundspeed", "GS", "number"),
      f("aircraft_type", "Type"),
    ],
    useRows: (p) => {
      const list = airports(p);
      const qs = useMultiAirportFlow(list);
      const rows = list.flatMap((ic, i) =>
        (qs[i]?.data?.flights ?? []).map((r) => ({ ...r, [AIRPORT_KEY]: ic })),
      ) as Row[];
      return {
        rows,
        isLoading: qs.some((q) => q.isLoading),
        isError: qs.length > 0 && qs.every((q) => q.isError),
      };
    },
  },
  {
    id: "departures",
    label: "Departures",
    category: "airport",
    needsIcao: true,
    fields: [
      f("callsign", "Callsign"),
      f("dep", "From"),
      f("arrival", "To"),
      f("aircraft_type", "Type"),
      f("status", "Status"),
      f("delay_min", "Delay", "number"),
      f("cfr", "CFR", "time"),
      f("gate", "Gate"),
      f("eta", "ETA", "time"),
      f("has_program", "Metered", "bool"),
    ],
    useRows: (p) => {
      const list = airports(p);
      const qs = useMultiDepartures(list);
      const rows = list.flatMap((ic, i) =>
        (qs[i]?.data?.departures ?? []).map((r) => ({ ...r, [AIRPORT_KEY]: ic })),
      ) as Row[];
      return {
        rows,
        isLoading: qs.some((q) => q.isLoading),
        isError: qs.length > 0 && qs.every((q) => q.isError),
      };
    },
  },
  {
    id: "taxi",
    label: "Taxi out",
    category: "airport",
    needsIcao: true,
    fields: [
      f("callsign", "Callsign"),
      f("dest", "Dest"),
      f("phase", "Phase"),
      f("gs", "GS", "number"),
      f("alt", "Alt", "number"),
    ],
    useRows: (p) => {
      const list = airports(p);
      const qs = useMultiTaxiStats(list);
      const rows = list.flatMap((ic, i) =>
        (qs[i]?.data?.active ?? []).map((r) => ({ ...r, [AIRPORT_KEY]: ic })),
      ) as Row[];
      return {
        rows,
        isLoading: qs.some((q) => q.isLoading),
        isError: qs.length > 0 && qs.every((q) => q.isError),
      };
    },
  },
  {
    id: "programs",
    label: "Metering programs",
    category: "global",
    needsIcao: false,
    fields: [
      f("icao", "Airport"),
      f("aar", "AAR", "number"),
      f("mit", "MIT", "number"),
      f("trail", "Trail", "number"),
      f("jets_only", "Jets only", "bool"),
      f("active_until", "Until", "time"),
      f("updated_by", "By"),
    ],
    useRows: () => {
      const q = usePrograms();
      return { rows: (q.data ?? []) as Row[], isLoading: q.isLoading, isError: q.isError };
    },
  },
  {
    id: "tmis",
    label: "TMIs",
    category: "global",
    needsIcao: false,
    fields: [
      f("requesting", "Requesting"),
      f("providing", "Providing"),
      f("restriction", "Restriction"),
      f("status", "Status"),
      f("start_time", "Start", "time"),
      f("stop_time", "Stop", "time"),
      f("author", "By"),
    ],
    useRows: () => {
      const q = useTmis();
      return { rows: (q.data ?? []) as Row[], isLoading: q.isLoading, isError: q.isError };
    },
  },
  {
    id: "fcas",
    label: "Flow constrained areas",
    category: "global",
    needsIcao: false,
    fields: [
      f("name", "Name"),
      f("artcc", "ARTCC"),
      f("rate", "Rate", "number"),
      f("mit", "MIT", "number"),
      f("mode", "Mode"),
      f("dir", "Dir"),
      f("min_fl", "Min FL", "number"),
      f("max_fl", "Max FL", "number"),
      f("enabled", "Enabled", "bool"),
    ],
    useRows: () => {
      const q = useFcas();
      return { rows: (q.data ?? []) as Row[], isLoading: q.isLoading, isError: q.isError };
    },
  },
  {
    id: "traffic",
    label: "Live traffic",
    category: "global",
    needsIcao: false,
    fields: [
      f("callsign", "Callsign"),
      f("dep", "From"),
      f("arr", "To"),
      f("actype", "Type"),
      f("alt", "Alt", "number"),
      f("gs", "GS", "number"),
      f("heading", "Hdg", "number"),
    ],
    useRows: () => {
      const q = useTraffic();
      return { rows: (q.data ?? []) as Row[], isLoading: q.isLoading, isError: q.isError };
    },
  },
];

export const DATA_SOURCES_BY_ID: Record<string, DataSource> = Object.fromEntries(
  DATA_SOURCES.map((s) => [s.id, s]),
);
