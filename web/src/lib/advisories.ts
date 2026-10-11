import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";
import {useToast} from "@ois/ui";

import {ois} from "./api";

export type Advisory = components["schemas"]["AdvisoryBody"];
export type CreateAdvisory = components["schemas"]["CreateAdvisoryRequest"];
export type UpdateAdvisory = components["schemas"]["UpdateAdvisoryRequest"];

/** The Reroute document type (#458) — the only `kind` with a structured form so far. */
export type Reroute = components["schemas"]["RerouteAdvisory"];
export type RerouteRoutes = components["schemas"]["RerouteRoutes"];
export type RerouteRow = components["schemas"]["RerouteRow"];
export type RerouteSegment = components["schemas"]["RerouteSegment"];
export type RerouteValidBasis = components["schemas"]["RerouteValidBasis"];

/** `kind` values the UI knows how to build a form for. Anything else is raw-only. */
export const ADVISORY_KIND_REROUTE = "reroute";

export function useAdvisories({ enabled = true }: { enabled?: boolean } = {}) {
  return useQuery({
    enabled,
    queryKey: ["advisories"],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/tmu/advisories");
      if (error || !data) throw new Error("failed to load advisories");
      return data;
    },
  });
}

export function useCreateAdvisory() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (body: CreateAdvisory) => {
      const { data, error } = await ois.POST("/api/v1/tmu/advisories", { body });
      if (error || !data) throw new Error("create failed");
      return data;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["advisories"] });
      toast.success("Draft advisory created");
    },
    onError: () => toast.error("Couldn’t create the advisory"),
  });
}

export function useUpdateAdvisory() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async ({ id, body }: { id: string; body: UpdateAdvisory }) => {
      const { data, error } = await ois.PATCH("/api/v1/tmu/advisories/{id}", {
        params: { path: { id } },
        body,
      });
      if (error || !data) throw new Error("update failed");
      return data;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["advisories"] });
      toast.success("Draft saved");
    },
    onError: () => toast.error("Couldn’t save the draft"),
  });
}

export function usePublishAdvisory() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (id: string) => {
      const { data, error } = await ois.POST("/api/v1/tmu/advisories/{id}/publish", {
        params: { path: { id } },
      });
      if (error || !data) throw new Error("publish failed");
      return data;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["advisories"] });
      toast.success("Advisory published");
    },
    onError: () => toast.error("Couldn’t publish the advisory"),
  });
}

export function useCancelAdvisory() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (id: string) => {
      const { data, error } = await ois.POST("/api/v1/tmu/advisories/{id}/cancel", {
        params: { path: { id } },
      });
      if (error || !data) throw new Error("cancel failed");
      return data;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["advisories"] });
      toast.success("Advisory cancelled");
    },
    onError: () => toast.error("Couldn’t cancel the advisory"),
  });
}

export function useDeleteAdvisory() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (id: string) => {
      const { error, response } = await ois.DELETE("/api/v1/tmu/advisories/{id}", {
        params: { path: { id } },
      });
      if (error) {
        const err = new Error("delete failed") as Error & { status?: number };
        err.status = response?.status;
        throw err;
      }
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["advisories"] });
      toast.success("Draft discarded");
    },
    // `delete_advisory` (backend/src/handlers/tmu.rs) answers 403 when the draft is outside the
    // caller's facility scope, 404 when it is gone, and 409 when it is no longer a draft: a published
    // advisory is a document that went out, so it is never deleted.
    onError: (error: Error & { status?: number }) => {
      toast.error(
        error.status === 403
          ? "You can’t discard drafts for this facility"
          : error.status === 404
            ? "That draft no longer exists"
            : error.status === 409
              ? "Only a draft can be discarded"
              : "Couldn’t discard the draft",
      );
    },
  });
}

/** An empty Reroute, for a fresh structured draft. `routes` starts as one blank single-segment row
 *  because `routes` is required — a structured advisory cannot be created without it. */
export const EMPTY_REROUTE: Reroute = {
  name: "",
  header: "",
  impacted_area: "",
  routes: { kind: "single", rows: [{ orig: "", dest: "", route: "" }] },
  valid: { basis: "fca_entry_time", from: "", to: "" },
};
