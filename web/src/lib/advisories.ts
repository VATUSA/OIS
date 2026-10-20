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

type StatusError = Error & { status?: number };

/** The thrown error carries the HTTP status, so `onError` can toast the reason. */
function statusError(message: string, response: Response | undefined): StatusError {
  const err: StatusError = new Error(message);
  err.status = response?.status;
  return err;
}

/**
 * The publish, cancel and delete handlers answer 403 outside the caller's facility scope, 404 for an
 * advisory that no longer exists, and 409 when its status does not allow the action.
 */
function reasonFor(
  error: StatusError,
  messages: { 403: string; 404: string; 409: string; other: string },
): string {
  return error.status === 403 || error.status === 404 || error.status === 409
    ? messages[error.status]
    : messages.other;
}

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
      const { data, error, response } = await ois.POST("/api/v1/tmu/advisories/{id}/publish", {
        params: { path: { id } },
      });
      if (error || !data) throw statusError("publish failed", response);
      return data;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["advisories"] });
      toast.success("Advisory published");
    },
    // `publish_advisory` (backend/src/handlers/tmu.rs) answers 409 when the advisory is not a draft.
    onError: (error: StatusError) =>
      toast.error(
        reasonFor(error, {
          403: "You can’t publish advisories for this facility",
          404: "That advisory no longer exists",
          409: "Only a draft can be published",
          other: "Couldn’t publish the advisory",
        }),
      ),
  });
}

export function useCancelAdvisory() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (id: string) => {
      const { data, error, response } = await ois.POST("/api/v1/tmu/advisories/{id}/cancel", {
        params: { path: { id } },
      });
      if (error || !data) throw statusError("cancel failed", response);
      return data;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["advisories"] });
      toast.success("Advisory cancelled");
    },
    // `cancel_advisory` (backend/src/handlers/tmu.rs) answers 409 when the advisory is already cancelled.
    onError: (error: StatusError) =>
      toast.error(
        reasonFor(error, {
          403: "You can’t cancel advisories for this facility",
          404: "That advisory no longer exists",
          409: "That advisory is already cancelled",
          other: "Couldn’t cancel the advisory",
        }),
      ),
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
      if (error) throw statusError("delete failed", response);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["advisories"] });
      toast.success("Draft discarded");
    },
    // `delete_advisory` (backend/src/handlers/tmu.rs) answers 403 when the draft is outside the
    // caller's facility scope, 404 when it is gone, and 409 when it is no longer a draft: a published
    // advisory is a document that went out, so it is never deleted.
    onError: (error: StatusError) =>
      toast.error(
        reasonFor(error, {
          403: "You can’t discard drafts for this facility",
          404: "That draft no longer exists",
          409: "Only a draft can be discarded",
          other: "Couldn’t discard the draft",
        }),
      ),
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
