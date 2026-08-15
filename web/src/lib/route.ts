import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";
import {useToast} from "@ois/ui";

import {ois} from "./api";

export type MapRoute = components["schemas"]["RouteBody"];
export type UpsertRoute = components["schemas"]["UpsertRouteRequest"];

/** All shared map routes (visible to anyone who can view the flow map). */
export function useRoutes() {
  return useQuery({
    queryKey: ["routes"],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/flow/routes");
      if (error || !data) throw new Error("failed to load routes");
      return data;
    },
  });
}

export function useCreateRoute() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (body: UpsertRoute) => {
      const { data, error } = await ois.POST("/api/v1/flow/routes", { body });
      if (error || !data) throw new Error("create failed");
      return data;
    },
    onSuccess: (data) => {
      queryClient.invalidateQueries({ queryKey: ["routes"] });
      if (data.unresolved.length > 0) {
        toast.warning(`Route created — ${data.unresolved.length} token(s) unresolved`, {
          description: data.unresolved.join(" "),
        });
      } else {
        toast.success("Route created");
      }
    },
    onError: () => toast.error("Couldn’t create the route"),
  });
}

export function useUpdateRoute() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async ({ id, body }: { id: string; body: UpsertRoute }) => {
      const { data, error } = await ois.PUT("/api/v1/flow/routes/{id}", {
        params: { path: { id } },
        body,
      });
      if (error || !data) throw new Error("save failed");
      return data;
    },
    onSuccess: (data) => {
      queryClient.invalidateQueries({ queryKey: ["routes"] });
      if (data.unresolved.length > 0) {
        toast.warning(`Saved — ${data.unresolved.length} token(s) unresolved`, {
          description: data.unresolved.join(" "),
        });
      }
    },
    onError: () => toast.error("Couldn’t save the route"),
  });
}

export function useDeleteRoute() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (id: string) => {
      const { error } = await ois.DELETE("/api/v1/flow/routes/{id}", {
        params: { path: { id } },
      });
      if (error) throw new Error("delete failed");
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["routes"] });
      toast.success("Route deleted");
    },
    onError: () => toast.error("Couldn’t delete the route"),
  });
}
