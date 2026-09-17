import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import type {DashboardState} from "@/features/dashboard/types";

import {ois} from "./api";

export type DashboardSummary = components["schemas"]["DashboardSummary"];
export type DashboardCollection = components["schemas"]["DashboardCollection"];
export type DashboardLibrary = components["schemas"]["DashboardLibrary"];
export type DashboardBody = components["schemas"]["DashboardBody"];
export type SharedDashboardBody = components["schemas"]["SharedDashboardBody"];

// The `data` blob is typed `Record<string, never>` in the generated client (opaque jsonb); it is
// really a DashboardState. Small casts keep the call sites readable.
type OpaqueJson = Record<string, never>;
const asJson = (v: DashboardState): OpaqueJson => v as unknown as OpaqueJson;

/** The caller's dashboards + collections. */
export function useDashboards({ enabled = true }: { enabled?: boolean } = {}) {
  return useQuery({
    enabled,
    queryKey: ["dashboards"],
    queryFn: async (): Promise<DashboardLibrary> => {
      const { data, error } = await ois.GET("/api/v1/dashboards");
      if (error || !data) return { dashboards: [], collections: [] };
      return data;
    },
    staleTime: 10_000,
  });
}

/** One owned board, with its DashboardState. */
export function useDashboard(id: string | null) {
  return useQuery({
    queryKey: ["dashboard", id],
    enabled: !!id,
    queryFn: async (): Promise<DashboardBody> => {
      const { data, error } = await ois.GET("/api/v1/dashboards/{id}", {
        params: { path: { id: id! } },
      });
      if (error || !data) throw new Error("dashboard not found");
      return data;
    },
  });
}

/** A shared board by slug (read-only, any signed-in viewer). */
export function useSharedDashboard(slug: string | null) {
  return useQuery({
    queryKey: ["dashboard-shared", slug],
    enabled: !!slug,
    retry: false,
    queryFn: async (): Promise<SharedDashboardBody> => {
      const { data, error } = await ois.GET("/api/v1/dashboards/shared/{slug}", {
        params: { path: { slug: slug! } },
      });
      if (error || !data) throw new Error("shared dashboard not found");
      return data;
    },
  });
}

export function useCreateDashboard() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (input: {
      name: string;
      data?: DashboardState;
      collection_id?: string | null;
    }): Promise<DashboardBody> => {
      const { data, error } = await ois.POST("/api/v1/dashboards", {
        body: {
          name: input.name,
          data: asJson(input.data ?? { version: 1, widgets: [], layout: [] }),
          collection_id: input.collection_id ?? null,
        },
      });
      if (error || !data) throw new Error("create failed");
      return data;
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ["dashboards"] }),
  });
}

export function useUpdateDashboard() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (input: {
      id: string;
      name?: string;
      data?: DashboardState;
      collection_id?: string | null;
    }): Promise<DashboardBody> => {
      const { data, error } = await ois.PUT("/api/v1/dashboards/{id}", {
        params: { path: { id: input.id } },
        body: {
          name: input.name,
          data: input.data ? asJson(input.data) : undefined,
          collection_id: input.collection_id,
        } as never,
      });
      if (error || !data) throw new Error("update failed");
      return data;
    },
    onSuccess: (board) => {
      qc.setQueryData(["dashboard", board.id], board);
      qc.invalidateQueries({ queryKey: ["dashboards"] });
    },
  });
}

export function useDeleteDashboard() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (id: string) => {
      const { error } = await ois.DELETE("/api/v1/dashboards/{id}", {
        params: { path: { id } },
      });
      if (error) throw new Error("delete failed");
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ["dashboards"] }),
  });
}

export function useShareDashboard() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (id: string): Promise<string> => {
      const { data, error } = await ois.POST("/api/v1/dashboards/{id}/share", {
        params: { path: { id } },
      });
      if (error || !data) throw new Error("share failed");
      return data.share_slug;
    },
    onSuccess: (_slug, id) => {
      qc.invalidateQueries({ queryKey: ["dashboard", id] });
      qc.invalidateQueries({ queryKey: ["dashboards"] });
    },
  });
}

export function useUnshareDashboard() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (id: string) => {
      const { error } = await ois.DELETE("/api/v1/dashboards/{id}/share", {
        params: { path: { id } },
      });
      if (error) throw new Error("unshare failed");
    },
    onSuccess: (_v, id) => {
      qc.invalidateQueries({ queryKey: ["dashboard", id] });
      qc.invalidateQueries({ queryKey: ["dashboards"] });
    },
  });
}

export function useCopyDashboard() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (slug: string): Promise<string> => {
      const { data, error } = await ois.POST("/api/v1/dashboards/shared/{slug}/copy", {
        params: { path: { slug } },
      });
      if (error || !data) throw new Error("copy failed");
      return data.id;
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ["dashboards"] }),
  });
}

export function useCreateCollection() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (name: string): Promise<DashboardCollection> => {
      const { data, error } = await ois.POST("/api/v1/dashboard-collections", {
        body: { name },
      });
      if (error || !data) throw new Error("create collection failed");
      return data;
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ["dashboards"] }),
  });
}

export function useRenameCollection() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (input: { id: string; name: string }) => {
      const { error } = await ois.PUT("/api/v1/dashboard-collections/{id}", {
        params: { path: { id: input.id } },
        body: { name: input.name },
      });
      if (error) throw new Error("rename collection failed");
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ["dashboards"] }),
  });
}

export function useDeleteCollection() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (id: string) => {
      const { error } = await ois.DELETE("/api/v1/dashboard-collections/{id}", {
        params: { path: { id } },
      });
      if (error) throw new Error("delete collection failed");
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ["dashboards"] }),
  });
}
