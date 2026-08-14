import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";
import {useToast} from "@ois/ui";

import {ois} from "./api";

export type RunwayBoard = components["schemas"]["RunwayBoard"];
export type RunwayEnd = components["schemas"]["RunwayEnd"];
export type RunwayArrival = components["schemas"]["RunwayArrival"];
export type RunwayDemand = components["schemas"]["RunwayDemand"];
export type RunwayRec = components["schemas"]["RunwayRec"];
export type RunwayConfigRequest = components["schemas"]["RunwayConfigRequest"];

/** The shared runway-balancer board for an airport, refreshed every 15s. */
export function useRunway(icao: string | null) {
  return useQuery({
    queryKey: ["runway", icao],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/flow/runway/{icao}", {
        params: { path: { icao: icao! } },
      });
      if (error || !data) throw new Error("failed to load runway board");
      return data;
    },
    enabled: !!icao,
    refetchInterval: 15_000,
  });
}

/** Save the shared runway config (requires flow.runway.update). */
export function useUpdateRunway(icao: string) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (body: RunwayConfigRequest) => {
      const { data, error } = await ois.PUT("/api/v1/flow/runway/{icao}", {
        params: { path: { icao } },
        body,
      });
      if (error || !data) throw new Error("save failed");
      return data;
    },
    onSuccess: (data) => queryClient.setQueryData(["runway", icao], data),
    onError: () => toast.error("Couldn’t save runway config"),
  });
}
