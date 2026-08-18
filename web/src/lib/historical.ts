// Historical ("time-machine") data hooks. Each mode-aware hook takes `at` (Unix epoch seconds, or
// null for live): when null it drives the existing live query; when set it hits the matching
// `/api/v1/stats/hist/*` endpoint, which reconstructs the network at that instant and runs the same
// compute function the live feed uses — so it returns the identical body type. Keeping the branch
// inside a single `useQueries`/`useQuery` call keeps the hook count stable (Rules of Hooks safe),
// which is why the dashboard data-source `useRows` functions can swap live↔historical freely.

import {useQueries, useQuery} from "@tanstack/react-query";

import {ois} from "./api";
import {fetchDepartures} from "./departures";
import {fetchFlow} from "./feed";

async function fetchHistFlow(icao: string, at: number) {
  const { data, error } = await ois.GET("/api/v1/stats/hist/flow/{icao}", {
    params: { path: { icao }, query: { at } },
  });
  if (error || !data) throw new Error("failed to load historical flow");
  return data;
}

async function fetchHistDepartures(dep: string, at: number) {
  const { data, error } = await ois.GET("/api/v1/stats/hist/departures/{dep}", {
    params: { path: { dep }, query: { at } },
  });
  if (error || !data) throw new Error("failed to load historical departures");
  return data;
}

async function fetchLiveTraffic() {
  const { data, error } = await ois.GET("/api/v1/flow/traffic");
  if (error || !data) throw new Error("failed to load traffic");
  return data;
}

async function fetchHistTraffic(at: number) {
  const { data, error } = await ois.GET("/api/v1/stats/hist/traffic", {
    params: { query: { at } },
  });
  if (error || !data) throw new Error("failed to load historical traffic");
  return data;
}

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
          },
    ),
  });
}

/** Pending departures per field — live (poll) or reconstructed at `at`. */
export function useModeDepartures(fields: string[], at: number | null) {
  return useQueries({
    queries: fields.map((dep) =>
      at == null
        ? { queryKey: ["departures", dep], queryFn: () => fetchDepartures(dep), refetchInterval: 20_000 }
        : {
            queryKey: ["hist-departures", dep, at],
            queryFn: () => fetchHistDepartures(dep, at),
            staleTime: Infinity,
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
  });
}
