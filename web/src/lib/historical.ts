// Historical ("time-machine") data hooks. Each mode-aware hook takes `at` (Unix epoch seconds, or
// null for live): when null it drives the existing live query; when set it hits the matching
// `/api/v1/stats/hist/*` endpoint, which reconstructs the network at that instant and runs the same
// compute function the live feed uses — so it returns the identical body type. Keeping the branch
// inside a single `useQueries`/`useQuery` call keeps the hook count stable (Rules of Hooks safe),
// which is why the dashboard data-source `useRows` functions can swap live↔historical freely.

import {keepPreviousData, useQueries, useQuery} from "@tanstack/react-query";

import {ois} from "./api";
import {fetchDepartures, fetchHistDepartures} from "./departures";
import {fetchFlow, fetchHistFlow} from "./feed";
import {fetchHistTaxi, fetchTaxi} from "./taxi";

async function fetchLiveTraffic() {
  const { data, error } = await ois.GET("/api/v1/flow/traffic");
  if (error || !data) throw new Error("failed to load traffic");
  return data;
}

export async function fetchHistTraffic(at: number) {
  const { data, error } = await ois.GET("/api/v1/stats/hist/traffic", {
    params: { query: { at } },
  });
  if (error || !data) throw new Error("failed to load historical traffic");
  return data;
}

// While replaying, every scrubber tick changes the query key (`…, at`). `keepPreviousData` keeps the
// last frame on screen while the next reconstructs, so the widgets update in place — no loading
// flash, no layout collapse. Combined with `staleTime: Infinity`, a second pass over the window is
// served entirely from cache and plays back instantly.

/** Arrival flow per airport — live (poll) or reconstructed at `at`. */
export function useModeAirportFlow(icaos: string[], at: number | null) {
  return useQueries({
    queries: icaos.map((icao) =>
      at == null
        ? { queryKey: ["flow", icao], queryFn: () => fetchFlow(icao), refetchInterval: 20_000 }
        : {
            queryKey: ["hist-flow", icao, at],
            queryFn: () => fetchHistFlow(icao, at),
            staleTime: Infinity,
            placeholderData: keepPreviousData,
          },
    ),
  });
}

/** Pending departures per field — live (poll) or reconstructed at `at`. */
export function useModeDepartures(fields: string[], at: number | null) {
  return useQueries({
    queries: fields.map((dep) =>
      at == null
        ? { queryKey: ["departures", dep], queryFn: () => fetchDepartures(dep), refetchInterval: 60_000 }
        : {
            queryKey: ["hist-departures", dep, at],
            queryFn: () => fetchHistDepartures(dep, at),
            staleTime: Infinity,
            placeholderData: keepPreviousData,
          },
    ),
  });
}

/** Taxi monitor per airport — live (poll) or replayed at `at`. */
export function useModeTaxi(icaos: string[], at: number | null) {
  return useQueries({
    queries: icaos.map((icao) =>
      at == null
        ? { queryKey: ["taxi", icao], queryFn: () => fetchTaxi(icao), refetchInterval: 15_000 }
        : {
            queryKey: ["hist-taxi", icao, at],
            queryFn: () => fetchHistTaxi(icao, at),
            staleTime: Infinity,
            placeholderData: keepPreviousData,
          },
    ),
  });
}

/** Map traffic — live (poll) or reconstructed at `at`. */
export function useModeTraffic(at: number | null) {
  return useQuery({
    queryKey: at == null ? ["flow-traffic"] : ["hist-traffic", at],
    queryFn: () => (at == null ? fetchLiveTraffic() : fetchHistTraffic(at)),
    refetchInterval: at == null ? 15_000 : false,
    staleTime: at == null ? 0 : Infinity,
    placeholderData: at == null ? undefined : keepPreviousData,
  });
}

/** Reconstructed pilot count at `at` for the "Pilots online" stat tile. Disabled (no fetch) when
 * live; shares the `["hist-traffic", at]` cache with a traffic widget on the same board. */
export function useHistPilotCount(at: number | null) {
  return useQuery({
    queryKey: ["hist-traffic", at],
    enabled: at != null,
    staleTime: Infinity,
    placeholderData: keepPreviousData,
    queryFn: () => fetchHistTraffic(at!),
    select: (d) => d.length,
  });
}
