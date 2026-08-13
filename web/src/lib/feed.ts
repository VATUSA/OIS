import {useQuery} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {ois} from "./api";

export type Flow = components["schemas"]["Flow"];
export type FlowFlight = components["schemas"]["FlowFlight"];
export type FeedStatus = components["schemas"]["FeedStatusBody"];

/** Feed health, refreshed every 30s. */
export function useFeedStatus() {
  return useQuery({
    queryKey: ["feed-status"],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/feed/status");
      if (error || !data) throw new Error("failed to load feed status");
      return data;
    },
    refetchInterval: 30_000,
  });
}

/** Live arrival flow for one airport, refreshed every 20s. */
export function useAirportFlow(icao: string) {
  return useQuery({
    queryKey: ["flow", icao],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/tmu/flow/{icao}", {
        params: { path: { icao } },
      });
      if (error || !data) throw new Error("failed to load flow");
      return data;
    },
    enabled: !!icao,
    refetchInterval: 20_000,
  });
}
