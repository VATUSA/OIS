import {useQueries, useQuery} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {ois} from "./api";

export type TaxiField = components["schemas"]["TaxiField"];
export type TaxiActive = components["schemas"]["TaxiActive"];

async function fetchTaxi(icao: string) {
  const { data, error } = await ois.GET("/api/v1/tmu/taxi/{icao}", {
    params: { path: { icao } },
  });
  if (error || !data) throw new Error("failed to load taxi stats");
  return data;
}

/** Live taxi monitor for one airport, refreshed every 15s. */
export function useTaxiStats(icao: string) {
  return useQuery({
    queryKey: ["taxi", icao],
    queryFn: () => fetchTaxi(icao),
    enabled: !!icao,
    refetchInterval: 15_000,
  });
}

/** Taxi monitor for several airports (the monitored set). */
export function useMultiTaxiStats(icaos: string[]) {
  return useQueries({
    queries: icaos.map((icao) => ({
      queryKey: ["taxi", icao],
      queryFn: () => fetchTaxi(icao),
      refetchInterval: 15_000,
    })),
  });
}
