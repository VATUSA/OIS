import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {ois} from "@/lib/api";
import {SOCKET_FALLBACK_MS} from "@/lib/realtime";

export type SectorLimits = components["schemas"]["SectorLimitsBody"];
export type SectorLimit = components["schemas"]["SectorLimitBody"];

/** The query key for one ARTCC's limits; `flow.sector_limits` invalidates every ARTCC's. */
export const sectorLimitsKey = (artcc: string) => ["sector-limits", artcc];

/**
 * An ARTCC's sectors with their occupancy limits (#722), and whether the viewer may set them. An
 * ARTCC with no sector data comes back with no sectors, never an error.
 */
export function useSectorLimits(artcc: string) {
  return useQuery({
    queryKey: sectorLimitsKey(artcc),
    enabled: artcc !== "",
    // The socket's `flow.sector_limits` nudge is the fast path; this covers a missed one.
    refetchInterval: SOCKET_FALLBACK_MS,
    queryFn: async (): Promise<SectorLimits> => {
      const { data, error } = await ois.GET("/api/v1/flow/sector-limits/{artcc}", {
        params: { path: { artcc } },
      });
      if (error || !data) throw new Error("failed to load sector limits");
      return data;
    },
  });
}

/**
 * Sets one sector's limit. Setting the default clears the override; the server ignores a value equal
 * to what is stored. Callers decide what counts as a change before calling — see `SectorLimitInput`.
 */
export function useSetSectorLimit(artcc: string) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (input: { sectorId: string; limit: number }): Promise<SectorLimit> => {
      const { data, error } = await ois.PUT("/api/v1/flow/sector-limits/{artcc}/{sector_id}", {
        params: { path: { artcc, sector_id: input.sectorId } },
        body: { limit: input.limit },
      });
      if (error || !data) throw new Error("failed to set the sector limit");
      return data;
    },
    onSuccess: (saved) => {
      // Show the saved value at once instead of the stale one until the refetch lands.
      qc.setQueryData<SectorLimits>(sectorLimitsKey(artcc), (prev) =>
        prev && {
          ...prev,
          sectors: prev.sectors.map((s) => (s.sector_id === saved.sector_id ? saved : s)),
        },
      );
      return qc.invalidateQueries({ queryKey: sectorLimitsKey(artcc) });
    },
  });
}
