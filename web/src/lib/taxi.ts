import {useQuery} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {ois} from "./api";
import {useHistoricalAt} from "./historical-context";

export type TaxiField = components["schemas"]["TaxiField"];
export type TaxiActive = components["schemas"]["TaxiActive"];

export async function fetchTaxi(icao: string) {
  const { data, error } = await ois.GET("/api/v1/tmu/taxi/{icao}", {
    params: { path: { icao } },
  });
  if (error || !data) throw new Error("failed to load taxi stats");
  return data;
}

export async function fetchHistTaxi(icao: string, at: number) {
  const { data, error } = await ois.GET("/api/v1/stats/hist/taxi/{icao}", {
    params: { path: { icao }, query: { at } },
  });
  if (error || !data) throw new Error("failed to load historical taxi");
  return data;
}

/** Taxi monitor for one airport. Live (15s poll) by default; inside a `HistoricalProvider` it
 * replays the taxi machine over the stored positions at the scrubber instant. */
export function useTaxiStats(icao: string) {
  const at = useHistoricalAt();
  return useQuery({
    queryKey: at == null ? ["taxi", icao] : ["hist-taxi", icao, at],
    queryFn: () => (at == null ? fetchTaxi(icao) : fetchHistTaxi(icao, at!)),
    enabled: !!icao,
    refetchInterval: at == null ? 15_000 : false,
    staleTime: at == null ? 0 : Infinity,
  });
}
