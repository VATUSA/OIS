import {useQuery} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {ois} from "./api";

export type NetworkPoint = components["schemas"]["NetworkPointBody"];
export type KeyCount = components["schemas"]["KeyCountBody"];
export type StatsAirport = components["schemas"]["StatsAirportBody"];
export type StatsFlightSummary = components["schemas"]["StatsFlightSummary"];
export type StatsFlightDetail = components["schemas"]["StatsFlightDetail"];

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
