import {keepPreviousData, useMutation, useQueries, useQuery, useQueryClient,} from "@tanstack/react-query";
import type {components} from "@ois/api-client";
import {useToast} from "@ois/ui";

import {ois} from "./api";
import {useHistoricalAt} from "./historical-context";
import {hhmmZulu} from "./time";

export async function fetchDepartures(dep: string) {
  const { data, error } = await ois.GET("/api/v1/tmu/departures/{dep}", {
    params: { path: { dep } },
  });
  if (error || !data) throw new Error("failed to load departures");
  return data;
}

export async function fetchHistDepartures(dep: string, at: number) {
  const { data, error } = await ois.GET("/api/v1/stats/hist/departures/{dep}", {
    params: { path: { dep }, query: { at } },
  });
  if (error || !data) throw new Error("failed to load historical departures");
  return data;
}

export type Departure = components["schemas"]["DepartureFlight"];
export type DeparturesResponse = components["schemas"]["DeparturesResponse"];

/** Pending departures out of a field. Live (20s poll) by default; inside a `HistoricalProvider` it
 * reconstructs the field at the scrubber instant instead. */
export function useDepartures(dep: string) {
  const at = useHistoricalAt();
  return useQuery({
    queryKey: at == null ? ["departures", dep] : ["hist-departures", dep, at],
    queryFn: () => (at == null ? fetchDepartures(dep) : fetchHistDepartures(dep, at!)),
    enabled: !!dep,
    // CFR/release/GDP changes push over the websocket; departures is mostly discrete, so a slow poll
    // is a sufficient fallback + status refresh.
    refetchInterval: at == null ? 60_000 : false,
    staleTime: at == null ? 0 : Infinity,
    placeholderData: at == null ? undefined : keepPreviousData,
  });
}

/** One departures query per field, for the personal multi-field dashboard. */
export function useMultiDepartures(fields: string[]) {
  return useQueries({
    queries: fields.map((dep) => ({
      queryKey: ["departures", dep],
      queryFn: () => fetchDepartures(dep),
      refetchInterval: 60_000,
    })),
  });
}

/** Issue (lock) a CFR: proposed wheels-up, or `readyTime` when supplied. */
export function useIssueCfr() {
  const queryClient = useQueryClient();
  const toast = useToast();
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
    onSuccess: (data) => {
      queryClient.invalidateQueries({ queryKey: ["departures"] });
      toast.success(`CFR issued · ${data.callsign}`, {
        description: `Wheels-up ${hhmmZulu(data.wheels_up)}`,
      });
    },
    onError: () => toast.error("Couldn’t issue the CFR"),
  });
}

export function useReleaseCfr() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (callsign: string) => {
      const { error } = await ois.DELETE("/api/v1/tmu/cfr/{callsign}", {
        params: { path: { callsign } },
      });
      if (error) throw new Error("release failed");
    },
    onSuccess: (_data, callsign) => {
      queryClient.invalidateQueries({ queryKey: ["departures"] });
      toast.success("CFR released", { description: callsign });
    },
    onError: () => toast.error("Couldn’t release the CFR"),
  });
}
