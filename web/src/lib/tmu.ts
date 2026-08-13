import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {ois} from "./api";

export type Tmi = components["schemas"]["TmiBody"];
export type CreateTmi = components["schemas"]["CreateTmiRequest"];

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
