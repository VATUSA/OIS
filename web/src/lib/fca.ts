import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";
import {useToast} from "@ois/ui";

import {ois} from "./api";

export type Fca = components["schemas"]["FcaBody"];
export type UpsertFca = components["schemas"]["UpsertFcaRequest"];
export type TrafficAircraft = components["schemas"]["TrafficAircraft"];

/** All FCAs (shared across controllers). */
export function useFcas() {
  return useQuery({
    queryKey: ["fcas"],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/flow/fcas");
      if (error || !data) throw new Error("failed to load FCAs");
      return data;
    },
  });
}

export function useCreateFca() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (body: UpsertFca) => {
      const { data, error } = await ois.POST("/api/v1/flow/fcas", { body });
      if (error || !data) throw new Error("create failed");
      return data;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["fcas"] });
      toast.success("FCA created");
    },
    onError: () => toast.error("Couldn’t create the FCA"),
  });
}

export function useUpdateFca() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async ({ id, body }: { id: string; body: UpsertFca }) => {
      const { data, error } = await ois.PUT("/api/v1/flow/fcas/{id}", {
        params: { path: { id } },
        body,
      });
      if (error || !data) throw new Error("save failed");
      return data;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["fcas"] });
    },
    onError: () => toast.error("Couldn’t save the FCA"),
  });
}

export function useDeleteFca() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (id: string) => {
      const { error } = await ois.DELETE("/api/v1/flow/fcas/{id}", {
        params: { path: { id } },
      });
      if (error) throw new Error("delete failed");
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["fcas"] });
      toast.success("FCA deleted");
    },
    onError: () => toast.error("Couldn’t delete the FCA"),
  });
}

/** Live VATSIM traffic for the map, refreshed every 15s. */
export function useTraffic() {
  return useQuery({
    queryKey: ["flow-traffic"],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/flow/traffic");
      if (error || !data) throw new Error("failed to load traffic");
      return data;
    },
    refetchInterval: 15_000,
  });
}

/** Build an upsert payload from an existing FCA (for toggles / edits). */
export function toUpsert(fca: Fca): UpsertFca {
  return {
    name: fca.name,
    color: fca.color,
    artcc: fca.artcc,
    points: fca.points,
    dests: fca.dests,
    origins: fca.origins,
    fixes: fca.fixes,
    scope: fca.scope,
    min_fl: fca.min_fl,
    max_fl: fca.max_fl,
    dir: fca.dir,
    mode: fca.mode,
    rate: fca.rate,
    mit: fca.mit,
    enabled: fca.enabled,
  };
}
