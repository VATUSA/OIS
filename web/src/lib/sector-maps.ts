import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";
import {useToast} from "@ois/ui";

import {ois} from "./api";

export type SectorMap = components["schemas"]["SectorMapBody"];
export type SectorMaps = components["schemas"]["SectorMapsBody"];

/** Query key for one ARTCC's Monitor Alert Parameters. */
export function sectorMapsKey(artcc: string) {
  return ["sector-maps", artcc] as const;
}

/**
 * The value an edit to a sector's MAP should write, or `null` to write nothing (#598). Only a positive
 * whole number that differs from `current` is written; empty, zero, negative, fractional or unchanged
 * input cancels — so a stray blur never writes, and a bad entry never touches an existing override.
 */
export function mapEdit(input: string, current: number): number | null {
  const text = input.trim();
  if (!/^\d+$/.test(text)) return null;
  const value = Number(text);
  return value > 0 && value !== current ? value : null;
}

/**
 * An ARTCC's sectors and their MAPs. `editable` comes from the server because writes are
 * ARTCC-scoped and `me.permissions` has no ARTCC dimension.
 */
export function useSectorMaps(artcc: string | undefined) {
  return useQuery({
    queryKey: sectorMapsKey(artcc ?? ""),
    enabled: !!artcc,
    queryFn: async () => {
      const {data, error} = await ois.GET("/api/v1/flow/monitor/{artcc}/maps", {
        params: {path: {artcc: artcc as string}},
      });
      if (error || !data) throw new Error("sector maps failed");
      return data;
    },
  });
}

/** Set one sector's MAP. */
export function useSetSectorMap(artcc: string) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async ({sectorId, map}: {sectorId: string; map: number}) => {
      const {error} = await ois.PUT("/api/v1/flow/monitor/{artcc}/maps/{sector_id}", {
        params: {path: {artcc, sector_id: sectorId}},
        body: {map},
      });
      if (error) throw new Error("set sector map failed");
    },
    onSuccess: () => queryClient.invalidateQueries({queryKey: sectorMapsKey(artcc)}),
    onError: () => toast.error("Couldn’t save the alert parameter"),
  });
}
