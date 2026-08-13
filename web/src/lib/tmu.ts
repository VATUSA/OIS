import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {ois} from "./api";

export type Tmi = components["schemas"]["TmiBody"];
export type CreateTmi = components["schemas"]["CreateTmiRequest"];
export type Program = components["schemas"]["ProgramBody"];
export type GateRule = components["schemas"]["GateRule"];
export type UpsertProgram = components["schemas"]["UpsertProgramRequest"];

export function useTmis() {
  return useQuery({
    queryKey: ["tmis"],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/tmu/tmis");
      if (error || !data) throw new Error("failed to load TMIs");
      return data;
    },
  });
}

export function useCreateTmi() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (body: CreateTmi) => {
      const { data, error } = await ois.POST("/api/v1/tmu/tmis", { body });
      if (error || !data) throw new Error("create failed");
      return data;
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["tmis"] }),
  });
}

function useTmiIdAction(
  verb: "publish" | "cancel",
): ReturnType<typeof useMutation<Tmi, Error, string>> {
  const queryClient = useQueryClient();
  return useMutation<Tmi, Error, string>({
    mutationFn: async (id: string) => {
      const { data, error } = await ois.POST(
        `/api/v1/tmu/tmis/{id}/${verb}` as "/api/v1/tmu/tmis/{id}/publish",
        { params: { path: { id } } },
      );
      if (error || !data) throw new Error(`${verb} failed`);
      return data;
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["tmis"] }),
  });
}

export const usePublishTmi = () => useTmiIdAction("publish");
export const useCancelTmi = () => useTmiIdAction("cancel");

export function useDeleteTmi() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (id: string) => {
      const { error } = await ois.DELETE("/api/v1/tmu/tmis/{id}", {
        params: { path: { id } },
      });
      if (error) throw new Error("delete failed");
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["tmis"] }),
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
  return useMutation({
    mutationFn: async ({ icao, body }: { icao: string; body: UpsertProgram }) => {
      const { data, error } = await ois.PUT("/api/v1/tmu/programs/{icao}", {
        params: { path: { icao } },
        body,
      });
      if (error || !data) throw new Error("save failed");
      return data;
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["tmu-programs"] }),
  });
}

export function useDeleteProgram() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (icao: string) => {
      const { error } = await ois.DELETE("/api/v1/tmu/programs/{icao}", {
        params: { path: { icao } },
      });
      if (error) throw new Error("delete failed");
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["tmu-programs"] }),
  });
}
