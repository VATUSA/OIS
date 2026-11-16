import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";
import {useToast} from "@ois/ui";

import {ois} from "./api";

export type RunwayBoard = components["schemas"]["RunwayBoard"];
export type RunwayEnd = components["schemas"]["RunwayEnd"];
export type RunwayArrival = components["schemas"]["RunwayArrival"];
export type RunwayDemand = components["schemas"]["RunwayDemand"];
export type RunwayRec = components["schemas"]["RunwayRec"];
export type SavedRunwayConfig = components["schemas"]["SavedRunwayConfig"];
export type CustomEnd = components["schemas"]["CustomEnd"];
export type RunwayConfigRequest = components["schemas"]["RunwayConfigRequest"];

/** Approach heading for each flow preset: WEST means landing westbound. */
export const PRESET_HEADING: Record<string, number> = { W: 270, E: 90, N: 360, S: 180 };
/** A preset activates every runway end within this many degrees of its heading. */
export const PRESET_TOLERANCE_DEG = 65;

/** Degrees between two headings, the short way round (0..=180). */
export function angleDiff(a: number, b: number): number {
  return Math.abs((((a - b) % 360) + 540) % 360 - 180);
}

/** Ids of the ends a preset activates; `OFF` or an unknown preset activates none. */
export function presetEnds(ends: RunwayEnd[], preset: string): string[] {
  const heading = PRESET_HEADING[preset];
  if (heading === undefined) return [];
  return ends.filter((e) => angleDiff(e.hdg, heading) <= PRESET_TOLERANCE_DEG).map((e) => e.id);
}

/**
 * STAR base name, the key a STAR rule is saved under: strip the trailing revision (`CAMRN4` →
 * `CAMRN`, `DOTSS2A` → `DOTSS`). The backend stores rule keys as sent and matches them against
 * `feed::runway::star_base`, so the two must agree; both are pinned on the same cases.
 */
export function starBase(name: string): string {
  return name.trim().toUpperCase().replace(/\d[A-Z]?$/, "");
}

/** The shared runway-balancer board for an airport, refreshed every 15s. */
export function useRunway(icao: string | null) {
  return useQuery({
    queryKey: ["runway", icao],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/flow/runway/{icao}", {
        params: { path: { icao: icao! } },
      });
      if (error || !data) throw new Error("failed to load runway board");
      return data;
    },
    enabled: !!icao,
    // The realtime topic is the fast path (#645/#646); this poll is the fallback for a signed-out
    // viewer, a dropped socket, or a second replica (#649), so it is relaxed from 15s, not removed.
    refetchInterval: 60_000,
  });
}

/** Save the shared runway config (requires flow.runway.update). */
export function useUpdateRunway(icao: string) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (body: RunwayConfigRequest) => {
      const { data, error } = await ois.PUT("/api/v1/flow/runway/{icao}", {
        params: { path: { icao } },
        body,
      });
      if (error || !data) throw new Error("save failed");
      return data;
    },
    onSuccess: (data) => queryClient.setQueryData(["runway", icao], data),
    onError: () => toast.error("Couldn’t save runway config"),
  });
}

/** Named runway configs saved for an airport. */
export function useSavedConfigs(icao: string | null) {
  return useQuery({
    queryKey: ["runway-configs", icao],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/flow/runway/{icao}/configs", {
        params: { path: { icao: icao! } },
      });
      if (error || !data) throw new Error("failed to load saved configs");
      return data;
    },
    enabled: !!icao,
  });
}

/** Save (or replace) a named runway config. */
export function useSaveConfig(icao: string) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (v: {
      name: string;
      active_ends: string[];
      star_rules: Record<string, string>;
    }) => {
      const { error } = await ois.PUT("/api/v1/flow/runway/{icao}/configs/{name}", {
        params: { path: { icao, name: v.name } },
        body: { active_ends: v.active_ends, star_rules: v.star_rules },
      });
      if (error) throw new Error("save failed");
    },
    onSuccess: (_d, v) => {
      queryClient.invalidateQueries({ queryKey: ["runway-configs", icao] });
      toast.success(`Saved “${v.name}”`);
    },
    onError: () => toast.error("Couldn’t save config"),
  });
}

/** Delete a named runway config. */
export function useDeleteConfig(icao: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (name: string) => {
      const { error } = await ois.DELETE("/api/v1/flow/runway/{icao}/configs/{name}", {
        params: { path: { icao, name } },
      });
      if (error) throw new Error("delete failed");
    },
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: ["runway-configs", icao] }),
  });
}
