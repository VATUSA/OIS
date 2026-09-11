import {useCallback, useEffect, useMemo, useRef, useState} from "react";
import {keepPreviousData, useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";
import {useToast} from "@ois/ui";

import {ois} from "./api";

export type NetworkPoint = components["schemas"]["NetworkPointBody"];
export type KeyCount = components["schemas"]["KeyCountBody"];
export type StatsAirport = components["schemas"]["StatsAirportBody"];
export type StatsFlightSummary = components["schemas"]["StatsFlightSummary"];
export type StatsFlightDetail = components["schemas"]["StatsFlightDetail"];
export type CaptureSummary = components["schemas"]["CaptureSummaryBody"];
export type SaveCapture = components["schemas"]["SaveCaptureRequest"];
export type StorageForecast = components["schemas"]["StorageForecastBody"];
export type Replay = components["schemas"]["ReplayBody"];
export type ReplayFlight = components["schemas"]["ReplayFlightBody"];
export type ResolvedRoute = components["schemas"]["ResolvedRoute"];
export type DelaySummary = components["schemas"]["DelaySummary"];
export type DelayGroup = components["schemas"]["DelayGroup"];
export type AtcBoard = components["schemas"]["AtcBoard"];

/** Average-delay aggregates for one leg kind over a rolling window, with optional filters. */
export function useDelaySummary(params: {
  kind: "departure" | "arrival";
  airport?: string;
  runway?: string;
  procedure?: string;
  hours: number;
}) {
  return useQuery({
    queryKey: ["stats-delays", params],
    queryFn: async (): Promise<DelaySummary> => {
      const { data, error } = await ois.GET("/api/v1/stats/delays", {
        params: {
          query: {
            kind: params.kind,
            airport: params.airport || undefined,
            runway: params.runway || undefined,
            procedure: params.procedure || undefined,
            hours: params.hours,
          },
        },
      });
      if (error || !data) throw new Error("failed to load delays");
      return data;
    },
  });
}

/** Sample spacing for a window length — mirrors the backend so chunks fetch a stable step. */
export function adaptiveStep(windowSecs: number): number {
  if (windowSecs <= 2 * 3600) return 15;
  if (windowSecs <= 6 * 3600) return 30;
  if (windowSecs <= 24 * 3600) return 60;
  if (windowSecs <= 72 * 3600) return 120;
  return 300;
}

export type ProgressiveReplay = {
  /** The replay assembled so far (grows as chunks arrive); undefined until the first chunk lands. */
  data?: Replay;
  isLoading: boolean;
  isError: boolean;
  /** Seconds from the window start that are loaded (contiguous from 0). */
  loadedUntil: number;
  /** A chunk fetch is in flight. */
  loading: boolean;
  /** Ask the loader to have data up to `untilSec` (from window start) available. Idempotent. */
  ensureLoaded: (untilSec: number) => void;
};

/**
 * Progressive replay loader: fetches positions in time chunks from `/stats/replay/positions`, so
 * playback can start on the first chunk and the rest streams in as the clock advances (or on scrub).
 * Only ever scans one chunk server-side, and picks an adaptive step so long windows stay bounded.
 */
export function useProgressiveReplay(from: number | null, to: number | null): ProgressiveReplay {
  const windowSecs = from != null && to != null ? Math.max(1, to - from) : 0;
  const step = useMemo(() => adaptiveStep(windowSecs), [windowSecs]);
  // ~240 buckets/chunk, bounded so a chunk is neither tiny nor enormous.
  const chunkSpan = useMemo(() => Math.min(24 * 3600, Math.max(15 * 60, step * 240)), [step]);

  const [flights, setFlights] = useState<Map<string, ReplayFlight>>(() => new Map());
  const [loadedUntil, setLoadedUntil] = useState(0);
  const [loading, setLoading] = useState(false);
  const [isError, setError] = useState(false);
  const [firstDone, setFirstDone] = useState(false);

  const busy = useRef(false);
  const loadedRef = useRef(0); // seconds loaded (contiguous from window start)
  const wantRef = useRef(0); // desired loaded-until (seconds)

  const pump = useCallback(async () => {
    if (from == null || to == null || busy.current) return;
    if (loadedRef.current >= wantRef.current || from + loadedRef.current >= to) return;
    busy.current = true;
    setLoading(true);
    const cfrom = from + loadedRef.current;
    const cto = Math.min(to, cfrom + chunkSpan);
    const { data, error } = await ois.GET("/api/v1/stats/replay/positions", {
      params: { query: { from, to, cfrom, cto, step } },
    });
    busy.current = false;
    if (error || !data) {
      setError(true);
      setLoading(false);
      return;
    }
    setFlights((prev) => {
      const next = new Map(prev);
      for (const f of data.flights) {
        const existing = next.get(f.session_id);
        // Chunks are contiguous and half-open, so samples append in order without overlap.
        if (existing) next.set(f.session_id, { ...existing, samples: existing.samples.concat(f.samples) });
        else next.set(f.session_id, { ...f });
      }
      return next;
    });
    loadedRef.current = cto - from;
    setLoadedUntil(cto - from);
    setFirstDone(true);
    setLoading(false);
    // Keep going if more was requested (e.g. a scrub jumped ahead).
    if (loadedRef.current < wantRef.current && from + loadedRef.current < to) void pump();
  }, [from, to, step, chunkSpan]);

  const ensureLoaded = useCallback(
    (untilSec: number) => {
      wantRef.current = Math.max(wantRef.current, untilSec);
      void pump();
    },
    [pump],
  );

  // Reset and kick the first chunk whenever the window changes.
  useEffect(() => {
    setFlights(new Map());
    loadedRef.current = 0;
    wantRef.current = 0;
    busy.current = false;
    setLoadedUntil(0);
    setFirstDone(false);
    setError(false);
    if (from != null && to != null) ensureLoaded(chunkSpan);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [from, to]);

  const data = useMemo<Replay | undefined>(() => {
    if (!firstDone || from == null || to == null) return undefined;
    return {
      capture_id: "",
      window_start: new Date(from * 1000).toISOString(),
      window_end: new Date(to * 1000).toISOString(),
      step_s: step,
      flights: Array.from(flights.values()),
    };
  }, [firstDone, flights, step, from, to]);

  return {
    data,
    isLoading: from != null && to != null && !firstDone && !isError,
    isError,
    loadedUntil,
    loading,
    ensureLoaded,
  };
}

/** Resolve a batch of filed routes to drawable polylines (for the replay map's route overlay). */
export async function resolveRoutes(
  entries: { callsign: string; dep: string; arr: string; route: string }[],
): Promise<ResolvedRoute[]> {
  if (entries.length === 0) return [];
  const { data, error } = await ois.POST("/api/v1/flow/resolve-routes", { body: entries });
  if (error || !data) throw new Error("failed to resolve routes");
  return data;
}

/** Hourly network totals over [from, to] (ISO strings; backend defaults to last 7d). */
export function useNetworkHistory(from: string, to: string) {
  return useQuery({
    queryKey: ["stats-network-history", from, to],
    queryFn: async (): Promise<NetworkPoint[]> => {
      const { data, error } = await ois.GET("/api/v1/stats/network/history", {
        params: { query: { from, to } },
      });
      if (error || !data) throw new Error("failed to load network history");
      return data;
    },
  });
}

export function useAirportsTop(limit = 15) {
  return useQuery({
    queryKey: ["stats-airports-top", limit],
    queryFn: async (): Promise<KeyCount[]> => {
      const { data, error } = await ois.GET("/api/v1/stats/airports/top", {
        params: { query: { limit } },
      });
      if (error || !data) throw new Error("failed to load top airports");
      return data;
    },
  });
}

export function useAirportStats(icao: string | null) {
  return useQuery({
    queryKey: ["stats-airport", icao],
    enabled: !!icao,
    queryFn: async (): Promise<StatsAirport> => {
      const { data, error } = await ois.GET("/api/v1/stats/airports/{icao}", {
        params: { path: { icao: icao! } },
      });
      if (error || !data) throw new Error("failed to load airport stats");
      return data;
    },
  });
}

/** Replayable capture windows (saved or open). */
export function useCaptures() {
  return useQuery({
    queryKey: ["stats-captures"],
    queryFn: async (): Promise<CaptureSummary[]> => {
      const { data, error } = await ois.GET("/api/v1/stats/captures");
      if (error || !data) throw new Error("failed to load captures");
      return data;
    },
  });
}

/** Save an already-viewed window as a permanent, named capture. */
export function useSaveCapture() {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (body: SaveCapture): Promise<CaptureSummary> => {
      const { data, error } = await ois.POST("/api/v1/stats/captures", { body });
      if (error || !data) throw new Error("failed to save capture");
      return data;
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["stats-captures"] });
      toast.success("Capture saved");
    },
    onError: () =>
      toast.error("Couldn’t save that window — it may already be past retention"),
  });
}

/** Current `stats` schema disk usage + a naive growth projection (admin/jobs page). */
export function useStorageForecast() {
  return useQuery({
    queryKey: ["stats-storage-forecast"],
    queryFn: async (): Promise<StorageForecast> => {
      const { data, error } = await ois.GET("/api/v1/stats/storage-forecast");
      if (error || !data) throw new Error("failed to load storage forecast");
      return data;
    },
  });
}

/** Per-flight tracks for replaying a capture window (fetched once, cached). */
export function useCaptureReplay(captureId: string | null, step = 30) {
  return useQuery({
    queryKey: ["stats-replay", captureId, step],
    enabled: !!captureId,
    staleTime: Infinity,
    queryFn: async (): Promise<Replay> => {
      const { data, error } = await ois.GET("/api/v1/stats/captures/{id}/replay", {
        params: { path: { id: captureId! }, query: { step } },
      });
      if (error || !data) throw new Error("failed to load replay");
      return data;
    },
  });
}

/** Per-flight tracks for replaying an arbitrary [from, to] window (Unix seconds), not tied to a
 * saved capture. Fetched once, cached. */
export function useWindowReplay(from: number | null, to: number | null, step = 30) {
  return useQuery({
    queryKey: ["stats-window-replay", from, to, step],
    enabled: from != null && to != null && to > from,
    staleTime: Infinity,
    queryFn: async (): Promise<Replay> => {
      const { data, error } = await ois.GET("/api/v1/stats/replay", {
        params: { query: { from: from!, to: to!, step } },
      });
      if (error || !data) throw new Error("failed to load replay");
      return data;
    },
  });
}

/** Online ATC at a replay instant (Unix seconds), bucketed to 30s so scrubbing/playback doesn't
 * refetch on every animation frame — matches the granularity replay data already runs at. */
export function useReplayAtc(atSeconds: number | null) {
  const bucketed = atSeconds == null ? null : Math.floor(atSeconds / 30) * 30;
  return useQuery({
    queryKey: ["stats-replay-atc", bucketed],
    enabled: bucketed != null,
    placeholderData: keepPreviousData, // avoid a blank overlay flash between buckets
    queryFn: async (): Promise<AtcBoard> => {
      const { data, error } = await ois.GET("/api/v1/stats/hist/atc", {
        params: { query: { at: bucketed! } },
      });
      if (error || !data) throw new Error("failed to load ATC");
      return data;
    },
  });
}

export function useFlightDetail(sessionId: string) {
  return useQuery({
    queryKey: ["stats-flight", sessionId],
    enabled: !!sessionId,
    queryFn: async (): Promise<StatsFlightDetail> => {
      const { data, error } = await ois.GET("/api/v1/stats/flights/{id}", {
        params: { path: { id: sessionId } },
      });
      if (error || !data) throw new Error("failed to load flight");
      return data;
    },
  });
}

export function useAirportMovements(icao: string | null, dir: "arr" | "dep", limit = 20) {
  return useQuery({
    queryKey: ["stats-airport-movements", icao, dir, limit],
    enabled: !!icao,
    queryFn: async (): Promise<StatsFlightSummary[]> => {
      const { data, error } = await ois.GET("/api/v1/stats/airports/{icao}/movements", {
        params: { path: { icao: icao! }, query: { dir, limit } },
      });
      if (error || !data) throw new Error("failed to load movements");
      return data;
    },
  });
}
