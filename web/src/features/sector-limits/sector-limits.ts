import {useMutation} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {ois} from "@/lib/api";

type SectorLimit = components["schemas"]["SectorLimitBody"];

/**
 * Sets one sector's limit (#722). Setting the default clears the override; the server ignores a value
 * equal to what is stored. Callers decide what counts as a change before calling — the Sector Monitor's MAP
 * input commits only a positive whole number that differs — and refetch what the limit judges.
 */
export function useSetSectorLimit(artcc: string) {
  return useMutation({
    mutationFn: async (input: { sectorId: string; limit: number }): Promise<SectorLimit> => {
      const { data, error } = await ois.PUT("/api/v1/flow/sector-limits/{artcc}/{sector_id}", {
        params: { path: { artcc, sector_id: input.sectorId } },
        body: { limit: input.limit },
      });
      if (error || !data) throw new Error("failed to set the sector limit");
      return data;
    },
  });
}
