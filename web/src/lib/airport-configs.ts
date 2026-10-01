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

/** Degrees between two bearings, the short way round (0..180). */
function arcDeg(a: number, b: number): number {
  const d = ((a - b) % 360 + 360) % 360;
  return Math.min(d, 360 - d);
}

/** How far `dir` is from a config's rule: 0 when the rule contains it, else to the nearer edge. */
function distanceToRule(dir: number, c: AirportConfig): number {
  return inWindRange(dir, c.wind_from_deg, c.wind_to_deg)
    ? 0
    : Math.min(arcDeg(dir, c.wind_from_deg), arcDeg(dir, c.wind_to_deg));
}

/**
 * The config a wind selects: the **closest** non-calm config, by distance to its wind rule — 0 when the
 * rule contains the direction, else the degrees to its nearer edge. A tie goes to the calm-default,
 * because equally close has no answer and the configured default beats resolving by name. Calm or
 * unknown wind takes the calm-default outright. Returns undefined if there are no configs.
 *
 * Hand-mirrors `favored_config` (`backend/src/repos/airport_configs.rs`), which carries the full
 * reasoning and is authoritative for departures (#510). The two must move together — this side had no
 * tests at all until #510 added them, so a drift would previously have been caught by nothing.
 */
export function matchConfig(
  configs: AirportConfig[],
  windDir: number | null | undefined,
): AirportConfig | undefined {
  const calm = () => configs.find((c) => c.calm_default);
  if (windDir == null) return calm() ?? configs[0];

  const candidates = configs.filter((c) => !c.calm_default);
  if (candidates.length === 0) return calm() ?? configs[0];

  const scored = candidates.map((c) => ({ c, d: distanceToRule(windDir, c) }));
  const best = Math.min(...scored.map((s) => s.d));
  const tied = scored.filter((s) => s.d === best);
  if (tied.length > 1) return calm() ?? tied[0].c;
  return tied[0].c;
}

/** A `{ key: runway }` rule map as the editor's `KEY=RUNWAY, KEY=RUNWAY` text. */
export function formatRules(rules: Record<string, string> | undefined | null): string {
  return Object.entries(rules ?? {})
    .map(([k, v]) => `${k}=${v}`)
    .join(", ");
}

/**
 * The editor's `KEY=RUNWAY` text back to a rule map (#512).
 *
 * Keys and runways are upper-cased, matching how the runway fields already normalise and how the
 * backend compares them — a rule typed `camrn=26r` is the same rule as `CAMRN=26R`, and treating them
 * as different would be a trap rather than a safeguard. A fragment with no `=`, or an empty half, is
 * dropped rather than stored as a rule pointing nowhere.
 */
export function parseRules(text: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const part of text.split(/[,\n]+/)) {
    const [rawKey, rawRunway] = part.split("=");
    const key = rawKey?.trim().toUpperCase();
    const runway = rawRunway?.trim().toUpperCase();
    if (key && runway) out[key] = runway;
  }
  return out;
}
