import {useQueries, useQuery} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {ois} from "./api";

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

async function fetchFlow(icao: string) {
  const { data, error } = await ois.GET("/api/v1/tmu/flow/{icao}", {
    params: { path: { icao } },
  });
  if (error || !data) throw new Error("failed to load flow");
  return data;
}

/** Live arrival flow for one airport, refreshed every 20s. */
export function useAirportFlow(icao: string) {
  return useQuery({
    queryKey: ["flow", icao],
    queryFn: () => fetchFlow(icao),
    enabled: !!icao,
    refetchInterval: 20_000,
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
