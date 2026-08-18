import {keepPreviousData, useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";
import {useToast} from "@ois/ui";

import {ois} from "./api";
import {useHistoricalAt} from "./historical-context";

export type Tmi = components["schemas"]["TmiBody"];
export type CreateTmi = components["schemas"]["CreateTmiRequest"];
export type Program = components["schemas"]["ProgramBody"];
export type GateRule = components["schemas"]["GateRule"];
export type UpsertProgram = components["schemas"]["UpsertProgramRequest"];
export type GroundStop = components["schemas"]["GroundStopBody"];
export type CreateGroundStop = components["schemas"]["CreateGroundStopRequest"];

export function useTmis() {
  const at = useHistoricalAt();
  return useQuery({
    queryKey: at == null ? ["tmis"] : ["hist-tmis", at],
    queryFn: async () => {
      if (at != null) {
        const { data, error } = await ois.GET("/api/v1/stats/hist/tmis", {
          params: { query: { at } },
        });
        if (error || !data) throw new Error("failed to load historical TMIs");
        return data;
      }
      const { data, error } = await ois.GET("/api/v1/tmu/tmis");
      if (error || !data) throw new Error("failed to load TMIs");
      return data;
    },
    staleTime: at == null ? undefined : Infinity,
    placeholderData: at == null ? undefined : keepPreviousData,
  });
}

export function useCreateTmi() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (body: CreateTmi) => {
      const { data, error } = await ois.POST("/api/v1/tmu/tmis", { body });
      if (error || !data) throw new Error("create failed");
      return data;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["tmis"] });
      toast.success("Restriction added");
    },
    onError: () => toast.error("Couldn’t add the restriction"),
  });
}

function useTmiIdAction(
  verb: "publish" | "cancel",
): ReturnType<typeof useMutation<Tmi, Error, string>> {
  const queryClient = useQueryClient();
  const toast = useToast();
  const past = verb === "publish" ? "published" : "cancelled";
  return useMutation<Tmi, Error, string>({
    mutationFn: async (id: string) => {
      const { data, error } = await ois.POST(
        `/api/v1/tmu/tmis/{id}/${verb}` as "/api/v1/tmu/tmis/{id}/publish",
        { params: { path: { id } } },
      );
      if (error || !data) throw new Error(`${verb} failed`);
      return data;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["tmis"] });
      toast.success(`Restriction ${past}`);
    },
    onError: () => toast.error(`Couldn’t ${verb} the restriction`),
  });
}

export const usePublishTmi = () => useTmiIdAction("publish");
export const useCancelTmi = () => useTmiIdAction("cancel");

export function useDeleteTmi() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (id: string) => {
      const { error } = await ois.DELETE("/api/v1/tmu/tmis/{id}", {
        params: { path: { id } },
      });
      if (error) throw new Error("delete failed");
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["tmis"] });
      toast.success("Restriction deleted");
    },
    onError: () => toast.error("Couldn’t delete the restriction"),
  });
}

// --- rate programs ---

export function usePrograms() {
  return useQuery({
    queryKey: ["tmu-programs"],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/tmu/programs");
      if (error || !data) throw new Error("failed to load programs");
      return data;
    },
  });
}

export function useUpsertProgram() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async ({ icao, body }: { icao: string; body: UpsertProgram }) => {
      const { data, error } = await ois.PUT("/api/v1/tmu/programs/{icao}", {
        params: { path: { icao } },
        body,
      });
      if (error || !data) throw new Error("save failed");
      return data;
    },
    onSuccess: (data) => {
      queryClient.invalidateQueries({ queryKey: ["tmu-programs"] });
      toast.success(`Program saved`, { description: data.icao });
    },
    onError: () => toast.error("Couldn’t save the program"),
  });
}

export function useDeleteProgram() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (icao: string) => {
      const { error } = await ois.DELETE("/api/v1/tmu/programs/{icao}", {
        params: { path: { icao } },
      });
      if (error) throw new Error("delete failed");
    },
    onSuccess: (_data, icao) => {
      queryClient.invalidateQueries({ queryKey: ["tmu-programs"] });
      toast.success("Program removed", { description: icao });
    },
    onError: () => toast.error("Couldn’t remove the program"),
  });
}

// --- ground stops ---

export function useGroundStops() {
  const at = useHistoricalAt();
  return useQuery({
    queryKey: at == null ? ["ground-stops"] : ["hist-ground-stops", at],
    queryFn: async () => {
      if (at != null) {
        const { data, error } = await ois.GET("/api/v1/stats/hist/ground-stops", {
          params: { query: { at } },
        });
        if (error || !data) throw new Error("failed to load historical ground stops");
        return data;
      }
      const { data, error } = await ois.GET("/api/v1/tmu/ground-stops");
      if (error || !data) throw new Error("failed to load ground stops");
      return data;
    },
    staleTime: at == null ? undefined : Infinity,
    placeholderData: at == null ? undefined : keepPreviousData,
  });
}

export function useCreateGroundStop() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (body: CreateGroundStop) => {
      const { data, error } = await ois.POST("/api/v1/tmu/ground-stops", { body });
      if (error || !data) throw new Error("create failed");
      return data;
    },
    onSuccess: (data) => {
      queryClient.invalidateQueries({ queryKey: ["ground-stops"] });
      toast.success("Ground stop added", { description: data.airport });
    },
    onError: () => toast.error("Couldn’t issue the ground stop"),
  });
}

function useGroundStopIdAction(
  verb: "publish" | "cancel",
): ReturnType<typeof useMutation<GroundStop, Error, string>> {
  const queryClient = useQueryClient();
  const toast = useToast();
  const past = verb === "publish" ? "published" : "cancelled";
  return useMutation<GroundStop, Error, string>({
    mutationFn: async (id: string) => {
      const { data, error } = await ois.POST(
        `/api/v1/tmu/ground-stops/{id}/${verb}` as "/api/v1/tmu/ground-stops/{id}/publish",
        { params: { path: { id } } },
      );
      if (error || !data) throw new Error(`${verb} failed`);
      return data;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["ground-stops"] });
      toast.success(`Ground stop ${past}`);
    },
    onError: () => toast.error(`Couldn’t ${verb} the ground stop`),
  });
}

export const usePublishGroundStop = () => useGroundStopIdAction("publish");
export const useCancelGroundStop = () => useGroundStopIdAction("cancel");

export function useDeleteGroundStop() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (id: string) => {
      const { error } = await ois.DELETE("/api/v1/tmu/ground-stops/{id}", {
        params: { path: { id } },
      });
      if (error) throw new Error("delete failed");
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["ground-stops"] });
      toast.success("Ground stop removed");
    },
    onError: () => toast.error("Couldn’t remove the ground stop"),
  });
}
