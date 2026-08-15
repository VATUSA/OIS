import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";
import {useToast} from "@ois/ui";

import {ois} from "./api";

export type Gdp = components["schemas"]["GdpBody"];
export type CreateGdp = components["schemas"]["CreateGdpRequest"];
export type UpdateGdp = components["schemas"]["UpdateGdpRequest"];
export type AarStep = components["schemas"]["AarStep"];
export type GdpBoard = components["schemas"]["GdpBoard"];
export type GdpFlightView = components["schemas"]["GdpFlightView"];
export type GdpDemand = components["schemas"]["GdpDemand"];

/** All Ground Delay Programs. */
export function useGdps() {
  return useQuery({
    queryKey: ["gdps"],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/tmu/gdp");
      if (error || !data) throw new Error("failed to load GDPs");
      return data;
    },
  });
}

/** The live board (RBS control times + demand) for one GDP, refreshed every 15s. */
export function useGdpBoard(id: string | null) {
  return useQuery({
    queryKey: ["gdp-board", id],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/tmu/gdp/{id}/board", {
        params: { path: { id: id! } },
      });
      if (error || !data) throw new Error("failed to load GDP board");
      return data;
    },
    enabled: !!id,
    refetchInterval: 15_000,
  });
}

/** Create a draft GDP. */
export function useCreateGdp() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (body: CreateGdp) => {
      const { data, error } = await ois.POST("/api/v1/tmu/gdp", { body });
      if (error || !data) throw new Error("create failed");
      return data;
    },
    onSuccess: (data) => {
      queryClient.invalidateQueries({ queryKey: ["gdps"] });
      toast.success("GDP created", { description: data.airport });
    },
    onError: () => toast.error("Couldn’t create the GDP"),
  });
}

/** Revise a GDP — change AAR/window/tier/scope. Re-rations a published program. */
export function useReviseGdp() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation<GdpBoard, Error, { id: string; body: UpdateGdp }>({
    mutationFn: async ({ id, body }) => {
      const { data, error } = await ois.PUT("/api/v1/tmu/gdp/{id}", {
        params: { path: { id } },
        body,
      });
      if (error || !data) throw new Error("revise failed");
      return data;
    },
    onSuccess: (data) => {
      queryClient.invalidateQueries({ queryKey: ["gdps"] });
      queryClient.setQueryData(["gdp-board", data.id], data);
      toast.success(
        data.published ? "GDP revised — control times reissued" : "GDP revised",
      );
    },
    onError: () => toast.error("Couldn’t revise the GDP"),
  });
}

/** Publish a GDP — freezes control times off the current feed. */
export function usePublishGdp() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation<GdpBoard, Error, string>({
    mutationFn: async (id: string) => {
      const { data, error } = await ois.POST("/api/v1/tmu/gdp/{id}/publish", {
        params: { path: { id } },
      });
      if (error || !data) throw new Error("publish failed");
      return data;
    },
    onSuccess: (data) => {
      queryClient.invalidateQueries({ queryKey: ["gdps"] });
      queryClient.setQueryData(["gdp-board", data.id], data);
      toast.success("GDP published — EDCTs frozen");
    },
    onError: () => toast.error("Couldn’t publish the GDP"),
  });
}

/** Cancel a draft or published GDP. */
export function useCancelGdp() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation<Gdp, Error, string>({
    mutationFn: async (id: string) => {
      const { data, error } = await ois.POST("/api/v1/tmu/gdp/{id}/cancel", {
        params: { path: { id } },
      });
      if (error || !data) throw new Error("cancel failed");
      return data;
    },
    onSuccess: (_d, id) => {
      queryClient.invalidateQueries({ queryKey: ["gdps"] });
      queryClient.invalidateQueries({ queryKey: ["gdp-board", id] });
      toast.success("GDP cancelled");
    },
    onError: () => toast.error("Couldn’t cancel the GDP"),
  });
}

/** Lock a controlled flight's advisory EDCT into a frozen slot. */
export function useLockSlot() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation<GdpBoard, Error, { id: string; callsign: string }>({
    mutationFn: async ({ id, callsign }) => {
      const { data, error } = await ois.POST(
        "/api/v1/tmu/gdp/{id}/slots/{callsign}",
        { params: { path: { id, callsign } } },
      );
      if (error || !data) throw new Error("lock failed");
      return data;
    },
    onSuccess: (data) => {
      queryClient.setQueryData(["gdp-board", data.id], data);
      toast.success("EDCT locked");
    },
    onError: () => toast.error("Couldn’t lock the EDCT"),
  });
}

/** Unlock a frozen slot — the flight reverts to an advisory control time. */
export function useUnlockSlot() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation<GdpBoard, Error, { id: string; callsign: string }>({
    mutationFn: async ({ id, callsign }) => {
      const { data, error } = await ois.DELETE(
        "/api/v1/tmu/gdp/{id}/slots/{callsign}",
        { params: { path: { id, callsign } } },
      );
      if (error || !data) throw new Error("unlock failed");
      return data;
    },
    onSuccess: (data) => {
      queryClient.setQueryData(["gdp-board", data.id], data);
      toast.success("EDCT unlocked");
    },
    onError: () => toast.error("Couldn’t unlock the EDCT"),
  });
}

/** Compress the program — reclaim freed capacity, pulling EDCTs earlier. */
export function useCompressGdp() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation<GdpBoard, Error, string>({
    mutationFn: async (id: string) => {
      const { data, error } = await ois.POST("/api/v1/tmu/gdp/{id}/compress", {
        params: { path: { id } },
      });
      if (error || !data) throw new Error("compress failed");
      return data;
    },
    onSuccess: (data) => {
      queryClient.setQueryData(["gdp-board", data.id], data);
      toast.success("Program compressed");
    },
    onError: () => toast.error("Couldn’t compress the program"),
  });
}

/** Delete a GDP. */
export function useDeleteGdp() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (id: string) => {
      const { error } = await ois.DELETE("/api/v1/tmu/gdp/{id}", {
        params: { path: { id } },
      });
      if (error) throw new Error("delete failed");
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["gdps"] });
      toast.success("GDP removed");
    },
    onError: () => toast.error("Couldn’t remove the GDP"),
  });
}
