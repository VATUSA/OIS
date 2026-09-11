import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {ois} from "./api";

export type AirportConfig = components["schemas"]["AirportConfigBody"];
export type UpsertAirportConfig = components["schemas"]["UpsertAirportConfigRequest"];
export type AirportForecast = components["schemas"]["AirportForecastBody"];

/** Reusable runway configs for an airport (default AAR/ADR + favored-wind rule). */
export function useAirportConfigs(icao: string | null) {
  return useQuery({
    queryKey: ["airport-configs", icao],
    enabled: !!icao,
    queryFn: async (): Promise<AirportConfig[]> => {
      const { data, error } = await ois.GET("/api/v1/airport-configs/{icao}", {
        params: { path: { icao: icao! } },
      });
      if (error || !data) throw new Error("failed to load configs");
      return data;
    },
  });
}

/** Every airport's runway configs, optionally scoped to one owning ARTCC (the all-airports list). */
export function useAllAirportConfigs(artcc: string | null) {
  return useQuery({
    queryKey: ["airport-configs", "all", artcc ?? null],
    queryFn: async (): Promise<AirportConfig[]> => {
      const { data, error } = await ois.GET("/api/v1/airport-configs", {
        params: { query: artcc ? { artcc } : {} },
      });
      if (error || !data) throw new Error("failed to load configs");
      return data;
    },
  });
}

export function useCreateAirportConfig(icao: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (body: UpsertAirportConfig): Promise<AirportConfig> => {
      const { data, error } = await ois.POST("/api/v1/airport-configs/{icao}", {
        params: { path: { icao } },
        body,
      });
      if (error || !data) throw new Error("create config failed");
      return data;
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ["airport-configs", icao] }),
  });
}

export function useUpdateAirportConfig(icao: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (input: { id: string; body: UpsertAirportConfig }): Promise<AirportConfig> => {
      const { data, error } = await ois.PUT("/api/v1/airport-configs/{icao}/{id}", {
        params: { path: { icao, id: input.id } },
        body: input.body,
      });
      if (error || !data) throw new Error("update config failed");
      return data;
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ["airport-configs", icao] }),
  });
}

export function useDeleteAirportConfig(icao: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (id: string) => {
      const { error } = await ois.DELETE("/api/v1/airport-configs/{icao}/{id}", {
        params: { path: { icao, id } },
      });
      if (error) throw new Error("delete config failed");
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ["airport-configs", icao] }),
  });
}

/** Forecast surface wind for an airport at a unix-seconds time (null time = skip). */
export function useForecast(icao: string | null, atUnix: number | null) {
  return useQuery({
    queryKey: ["forecast", icao, atUnix],
    enabled: !!icao && atUnix != null,
    staleTime: 30 * 60_000,
    queryFn: async (): Promise<AirportForecast> => {
      const { data, error } = await ois.GET("/api/v1/forecast/{icao}", {
        params: { path: { icao: icao! }, query: { at: atUnix! } },
      });
      if (error || !data) throw new Error("failed to load forecast");
      return data;
    },
  });
}

/** True if `dir` falls within [from, to] degrees (inclusive, wrap-around allowed). */
export function inWindRange(dir: number, from: number, to: number): boolean {
  return from <= to ? dir >= from && dir <= to : dir >= from || dir <= to;
}

/**
 * The config a forecast wind selects: the first non-calm config whose wind rule contains the
 * direction, else the calm-default (or the first config). Returns undefined if there are none.
 */
export function matchConfig(
  configs: AirportConfig[],
  windDir: number | null | undefined,
): AirportConfig | undefined {
  if (windDir != null) {
    const m = configs.find((c) => !c.calm_default && inWindRange(windDir, c.wind_from_deg, c.wind_to_deg));
    if (m) return m;
  }
  return configs.find((c) => c.calm_default) ?? configs[0];
}
