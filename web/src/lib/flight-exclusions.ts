import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";
import {useToast} from "@ois/ui";

import {ois} from "./api";

export type FlightExclusion = components["schemas"]["FlightExclusionBody"];

/** Query key for one FCA's active manual exclusions. */
export function exclusionsKey(fcaId: string) {
  return ["fca-exclusions", fcaId] as const;
}

/**
 * The bogus flights this FCA's facility has currently dropped (#342). Backs the panel that makes a
 * removal reversible rather than a black hole — without it an excluded flight is simply gone, with
 * nowhere to notice or undo it.
 */
export function useFlightExclusions(fcaId: string | undefined) {
  return useQuery({
    queryKey: exclusionsKey(fcaId ?? ""),
    enabled: !!fcaId,
    queryFn: async () => {
      const {data, error} = await ois.GET("/api/v1/flow/fcas/{id}/exclusions", {
        params: {path: {id: fcaId as string}},
      });
      if (error || !data) throw new Error("exclusions failed");
      return data;
    },
  });
}

/**
 * Every query the flight's disappearance has to reach: this FCA's strips and exclusion list, plus
 * the map traffic, badge counts and boards that also filter it out server-side.
 */
function invalidateAffected(
  queryClient: ReturnType<typeof useQueryClient>,
  fcaId: string,
) {
  queryClient.invalidateQueries({queryKey: exclusionsKey(fcaId)});
  queryClient.invalidateQueries({queryKey: ["fca-traffic"]});
  queryClient.invalidateQueries({queryKey: ["fca-counts"]});
  // The live map dots (`useTraffic`). Deliberately not ["hist-traffic", …]: a removal now must
  // not redraw a historical replay.
  queryClient.invalidateQueries({queryKey: ["flow-traffic"]});
  queryClient.invalidateQueries({queryKey: ["idst"]});
  queryClient.invalidateQueries({queryKey: ["departures"]});
}

/** Drop a bogus flight for this FCA's facility. */
export function useExcludeFlight(fcaId: string) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async ({
      callsign,
      reason = "",
    }: {
      callsign: string;
      reason?: string;
    }) => {
      const {data, error} = await ois.POST(
        "/api/v1/flow/fcas/{id}/exclusions/{callsign}",
        {params: {path: {id: fcaId, callsign}}, body: {reason}},
      );
      if (error || !data) throw new Error("exclude failed");
      return data;
    },
    onSuccess: (data) => {
      invalidateAffected(queryClient, fcaId);
      toast.success(`${data.callsign} removed`, {
        description: "Restore it from Removed flights.",
      });
    },
    onError: () => toast.error("Couldn’t remove the flight"),
  });
}

/** Put a mistakenly removed flight back. */
export function useRestoreFlight(fcaId: string) {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (callsign: string) => {
      const {error} = await ois.DELETE(
        "/api/v1/flow/fcas/{id}/exclusions/{callsign}",
        {params: {path: {id: fcaId, callsign}}},
      );
      if (error) throw new Error("restore failed");
      return callsign;
    },
    onSuccess: (callsign) => {
      invalidateAffected(queryClient, fcaId);
      toast.success(`${callsign} restored`);
    },
    onError: () => toast.error("Couldn’t restore the flight"),
  });
}
