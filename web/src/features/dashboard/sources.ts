// The data-source registry: each source wraps an existing React Query hook and exposes a flat
// `rows[]` plus a typed `fields[]` descriptor. Table (and later chart) widgets consume this, so
// adding a source here lights it up everywhere. Every `useRows` is a real hook — a widget binds
// one source for its lifetime, so calling `source.useRows(...)` unconditionally is hooks-safe
// (the TableWidget also keys its inner component by source id, remounting on any change).

import {useMemo} from "react";

import {useFcas} from "@/lib/fca";
import {useModeAirportFlow, useModeDepartures, useModeTaxi, useModeTraffic} from "@/lib/historical";
import {usePrograms, useTmis} from "@/lib/tmu";

import {useHistoricalAt} from "./historical";

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
  /** A background refetch is in flight (initial load or a manual/interval refresh). */
  isFetching: boolean;
  /** ms epoch of the most recent successful load, or 0 if never. */
  dataUpdatedAt: number;
  /** Force an immediate refetch of this source's underlying query/queries. */
  refetch: () => void;
}

/** Fold an array of React Query results (multi-airport sources) into RowsResult status fields. */
function multiStatus<T extends { isFetching: boolean; dataUpdatedAt: number; refetch: () => void }>(
  qs: T[],
): Pick<RowsResult, "isFetching" | "dataUpdatedAt" | "refetch"> {
  return {
    isFetching: qs.some((q) => q.isFetching),
    dataUpdatedAt: qs.reduce((m, q) => Math.max(m, q.dataUpdatedAt), 0),
    refetch: () => qs.forEach((q) => void q.refetch()),
  };
}

/** Status fields for a single-query (global) source. */
function singleStatus(q: {
  isFetching: boolean;
  dataUpdatedAt: number;
  refetch: () => void;
}): Pick<RowsResult, "isFetching" | "dataUpdatedAt" | "refetch"> {
  return { isFetching: q.isFetching, dataUpdatedAt: q.dataUpdatedAt, refetch: () => void q.refetch() };
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

/** Config/global sources with no historical form show nothing in historical mode (`at` set),
 * rather than mixing current config into a past view. Live (`at == null`) passes rows through. */
const histBlank = (at: number | null, data: unknown): Row[] =>
  at == null ? ((data ?? []) as Row[]) : [];

/** Normalize params to a concrete airport list (icaos preferred, else the single icao). */
const airports = (p: SourceParams): string[] => (p.icaos?.length ? p.icaos : p.icao ? [p.icao] : []);

/**
 * Flatten a multi-airport query result into tagged rows, memoized so the array keeps the SAME
 * reference between data updates. This is load-bearing: the rows feed a table widget's
 * `useReactTable({ data })`, and handing it a fresh array every render sends TanStack Table into a
 * re-render loop (it rebuilds its row model, re-renders, gets a new array, repeats) that pegs the
 * main thread and freezes the tab. We key the memo on the airport list + each query's
 * `dataUpdatedAt`, so rows only rebuild when the underlying data actually changes.
 */
function useTaggedRows<T>(
  list: string[],
  qs: { data?: T; dataUpdatedAt: number }[],
  pick: (data: T) => readonly unknown[],
): Row[] {
  const sig = `${list.join(",")}|${qs.map((q) => q?.dataUpdatedAt ?? 0).join(",")}`;
  return useMemo(
    () =>
      list.flatMap((ic, i) => {
        const d = qs[i]?.data;
        return (d ? pick(d) : []).map((r) => ({ ...(r as object), [AIRPORT_KEY]: ic }) as Row);
      }),
    [sig],
  );
}

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
      const at = useHistoricalAt();
      const list = airports(p);
      const qs = useModeAirportFlow(list, at);
      const rows = useTaggedRows(list, qs, (d) => d.flights ?? []);
      return {
        rows,
        isLoading: qs.some((q) => q.isLoading),
        isError: qs.length > 0 && qs.every((q) => q.isError),
        ...multiStatus(qs),
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
      const at = useHistoricalAt();
      const list = airports(p);
      const qs = useModeDepartures(list, at);
      const rows = useTaggedRows(list, qs, (d) => d.departures ?? []);
      return {
        rows,
        isLoading: qs.some((q) => q.isLoading),
        isError: qs.length > 0 && qs.every((q) => q.isError),
        ...multiStatus(qs),
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
      const at = useHistoricalAt();
      const list = airports(p);
      const qs = useModeTaxi(list, at);
      const rows = useTaggedRows(list, qs, (d) => d.active ?? []);
      return {
        rows,
        isLoading: qs.some((q) => q.isLoading),
        isError: qs.length > 0 && qs.every((q) => q.isError),
        ...multiStatus(qs),
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
      const at = useHistoricalAt();
      const q = usePrograms();
      return { rows: histBlank(at, q.data), isLoading: at == null && q.isLoading, isError: at == null && q.isError, ...singleStatus(q) };
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
      // TMIs are historized — useTmis is mode-aware, so this replays at the scrubber instant.
      const q = useTmis();
      return { rows: (q.data ?? []) as Row[], isLoading: q.isLoading, isError: q.isError, ...singleStatus(q) };
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
      // FCAs are historized — useFcas is mode-aware, so this replays at the scrubber instant.
      const q = useFcas();
      return { rows: (q.data ?? []) as Row[], isLoading: q.isLoading, isError: q.isError, ...singleStatus(q) };
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
      const at = useHistoricalAt();
      const q = useModeTraffic(at);
      return { rows: (q.data ?? []) as Row[], isLoading: q.isLoading, isError: q.isError, ...singleStatus(q) };
    },
  },
];

export const DATA_SOURCES_BY_ID: Record<string, DataSource> = Object.fromEntries(
  DATA_SOURCES.map((s) => [s.id, s]),
);
