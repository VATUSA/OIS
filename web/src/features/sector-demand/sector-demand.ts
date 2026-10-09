import {useQuery} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {ois} from "@/lib/api";
import {pollUnlessLive, useRealtimeLive} from "@/lib/realtime";

export type SectorDemand = components["schemas"]["SectorDemandBody"];
export type SectorDemandTable = components["schemas"]["SectorDemandTable"];
export type SectorDemandRow = components["schemas"]["SectorDemandRow"];
export type SectorDemandBin = components["schemas"]["SectorDemandBin"];

/**
 * The query key for one ARTCC's sector demand. `feed.tick`, `flow.sector_limits`,
 * `flow.sector_consolidations`, `flow.release`, `flow.cfr`, `tmu.gdp` and `flow.fca` invalidate every ARTCC's
 * (`web/src/lib/realtime.ts`).
 */
export const sectorDemandKey = (artcc: string) => ["sector-demand", artcc];

/**
 * An ARTCC's predicted sector demand (#725): an enroute and a TRACON table over six hours of Zulu
 * quarter-hours, each bin already judged against its row's limit by the server. `status` says when
 * there is nothing to draw (`no_sector_data`, `pending`). The server always sends all six hours; the
 * range a view draws is sliced client-side, with no refetch.
 *
 * `enabled: false` (a collapsed neighbour table) fetches nothing.
 */
export function useSectorDemand(artcc: string, { enabled = true }: { enabled?: boolean } = {}) {
  const live = useRealtimeLive();
  return useQuery({
    queryKey: sectorDemandKey(artcc),
    enabled: enabled && artcc !== "",
    // Off while feed ticks arrive (#648); the tick refetches this at most once a minute.
    refetchInterval: pollUnlessLive(60_000, live),
    queryFn: async (): Promise<SectorDemand> => {
      const { data, error } = await ois.GET("/api/v1/flow/sector-demand/{artcc}", {
        params: { path: { artcc } },
      });
      if (error || !data) throw new Error("failed to load sector demand");
      return data;
    },
  });
}
