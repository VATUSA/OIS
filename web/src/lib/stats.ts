import {useQuery} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {ois} from "./api";

export type NetworkPoint = components["schemas"]["NetworkPointBody"];
export type KeyCount = components["schemas"]["KeyCountBody"];
export type StatsAirport = components["schemas"]["StatsAirportBody"];
export type StatsFlightSummary = components["schemas"]["StatsFlightSummary"];
export type StatsFlightDetail = components["schemas"]["StatsFlightDetail"];
export type CaptureSummary = components["schemas"]["CaptureSummaryBody"];
export type Replay = components["schemas"]["ReplayBody"];
export type ReplayFlight = components["schemas"]["ReplayFlightBody"];

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
