import {useQueries, useQuery} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {ois} from "./api";
import {useHistoricalAt} from "./historical-context";

export type Flow = components["schemas"]["Flow"];
export type FlowFlight = components["schemas"]["FlowFlight"];
export type FeedStatus = components["schemas"]["FeedStatusBody"];

/**
 * The feed is stale if the last successful fetch errored (`healthy` false) or the last
 * ingest is older than `staleMs`. The age check catches a silently-hung poller, which
 * keeps `healthy` true but stops advancing `last_updated`.
 */
export function feedIsStale(
  status: Pick<FeedStatus, "healthy" | "last_updated"> | undefined,
  now: number,
  staleMs: number,
): boolean {
  if (!status) return false;
  if (!status.healthy) return true;
  const age = status.last_updated
    ? now - new Date(status.last_updated).getTime()
    : Infinity;
  return age > staleMs;
}

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

export async function fetchFlow(icao: string) {
  const { data, error } = await ois.GET("/api/v1/tmu/flow/{icao}", {
    params: { path: { icao } },
  });
  if (error || !data) throw new Error("failed to load flow");
  return data;
}

export async function fetchHistFlow(icao: string, at: number) {
  const { data, error } = await ois.GET("/api/v1/stats/hist/flow/{icao}", {
    params: { path: { icao }, query: { at } },
  });
  if (error || !data) throw new Error("failed to load historical flow");
  return data;
}

/** Arrival flow for one airport. Live (20s poll) by default; inside a `HistoricalProvider` it
 * reconstructs the flow at the scrubber instant instead (so airport-view widgets replay). */
export function useAirportFlow(icao: string) {
  const at = useHistoricalAt();
  return useQuery({
    queryKey: at == null ? ["flow", icao] : ["hist-flow", icao, at],
    queryFn: () => (at == null ? fetchFlow(icao) : fetchHistFlow(icao, at!)),
    enabled: !!icao,
    refetchInterval: at == null ? 20_000 : false,
    staleTime: at == null ? 0 : Infinity,
  });
}

/** Arrival flow for several airports at once (for comparison charts). */
export function useMultiAirportFlow(icaos: string[]) {
  return useQueries({
    queries: icaos.map((icao) => ({
      queryKey: ["flow", icao],
      queryFn: () => fetchFlow(icao),
      refetchInterval: 20_000,
    })),
  });
}
