import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {ois} from "./api";

export type Departure = components["schemas"]["DepartureFlight"];
export type DeparturesResponse = components["schemas"]["DeparturesResponse"];

/** Pending departures out of a field into any metered destination, refreshed every 20s. */
export function useDepartures(dep: string) {
  return useQuery({
    queryKey: ["departures", dep],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/tmu/departures/{dep}", {
        params: { path: { dep } },
      });
      if (error || !data) throw new Error("failed to load departures");
      return data;
    },
    enabled: !!dep,
    refetchInterval: 20_000,
  });
}

/** Issue (lock) a CFR: proposed wheels-up, or `readyTime` when supplied. */
export function useIssueCfr() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (args: {
      callsign: string;
      airport: string;
      readyTime?: string;
    }) => {
      const { data, error } = await ois.POST("/api/v1/tmu/cfr", {
        body: {
          callsign: args.callsign,
          airport: args.airport,
          ready_time: args.readyTime ?? null,
        },
      });
      if (error || !data) throw new Error("issue failed");
      return data;
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["departures"] }),
  });
}

export function useReleaseCfr() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (callsign: string) => {
      const { error } = await ois.DELETE("/api/v1/tmu/cfr/{callsign}", {
        params: { path: { callsign } },
      });
      if (error) throw new Error("release failed");
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["departures"] }),
  });
}
