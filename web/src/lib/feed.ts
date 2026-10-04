import {pollUnlessLive, useRealtimeLive} from "@/lib/realtime";
import {keepPreviousData, useQueries, useQuery} from "@tanstack/react-query";
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
export function useFeedStatus({ background = false }: { background?: boolean } = {}) {
  const live = useRealtimeLive();
  return useQuery({
    queryKey: ["feed-status"],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/feed/status");
      if (error || !data) throw new Error("failed to load feed status");
      return data;
    },
    // Off while feed ticks arrive (#648); the tick refetches this once per upstream publish.
    refetchInterval: pollUnlessLive(30_000, live),
    // TanStack skips a `refetchInterval` tick while the document is hidden. The menu-bar tray (#351)
    // reads this while the window is hidden to it — the one time its numbers matter — so it opts in.
    refetchIntervalInBackground: background,
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
  const live = useRealtimeLive();
  const at = useHistoricalAt();
  return useQuery({
    queryKey: at == null ? ["flow", icao] : ["hist-flow", icao, at],
    queryFn: () => (at == null ? fetchFlow(icao) : fetchHistFlow(icao, at!)),
    enabled: !!icao,
    // Off while feed ticks arrive (#648); the tick refetches this once per upstream publish.
    refetchInterval: at == null ? pollUnlessLive(20_000, live) : false,
    staleTime: at == null ? 0 : Infinity,
    // Replay: keep the last frame on screen while the next `at` reconstructs (no loading flash).
    placeholderData: at == null ? undefined : keepPreviousData,
  });
}

/** Arrival flow for several airports at once (for comparison charts). */
export function useMultiAirportFlow(icaos: string[]) {
  const live = useRealtimeLive();
  return useQueries({
    queries: icaos.map((icao) => ({
      queryKey: ["flow", icao],
      queryFn: () => fetchFlow(icao),
      // Off while feed ticks arrive (#648); the tick refetches this once per upstream publish.
      refetchInterval: pollUnlessLive(20_000, live),
    })),
  });
}
