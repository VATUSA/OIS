import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {useToast} from "@ois/ui";
import {ois} from "./api";

export type AceRequest = components["schemas"]["AceRequestBody"];
export type AceClaim = components["schemas"]["AceClaimBody"];
export type CreateAceRequest = components["schemas"]["CreateAceRequestRequest"];
export type ClaimAceRequest = components["schemas"]["ClaimAceRequest"];

/** The ACE support requests for one event (optionally filtered by status). Needs `ace.requests.read`. */
export function useEventAce(eventId: number, status?: string) {
  return useQuery({
    queryKey: ["event-ace", eventId, status ?? "all"],
    queryFn: async (): Promise<AceRequest[]> => {
      const { data, error } = await ois.GET("/api/v1/events/{id}/ace", {
        params: {
          path: { id: eventId },
          query: status ? { status } : {},
        },
      });
      if (error || !data) throw new Error("failed to load ACE requests");
      return data;
    },
    enabled: Number.isFinite(eventId),
    refetchInterval: 30_000,
  });
}

/** Open an ACE support request on an event. Needs `ace.requests.create`. */
export function useCreateEventAce(eventId: number) {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (body: CreateAceRequest): Promise<AceRequest> => {
      const { data, error } = await ois.POST("/api/v1/events/{id}/ace", {
        params: { path: { id: eventId } },
        body,
      });
      if (error || !data) throw new Error("create failed");
      return data;
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["event-ace", eventId] });
      toast.success("ACE support requested");
    },
    onError: () => toast.error("Couldn’t submit the request"),
  });
}

/** Delete an ACE request. Needs `ace.requests.decide`. */
export function useDeleteEventAce(eventId: number) {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (req: string): Promise<void> => {
      const { error } = await ois.DELETE("/api/v1/events/{id}/ace/{req}", {
        params: { path: { id: eventId, req } },
      });
      if (error) throw new Error("delete failed");
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["event-ace", eventId] });
      toast.success("Request removed");
    },
    onError: () => toast.error("Couldn’t remove the request"),
  });
}

/** Claim a slot on an open request. Needs `ace.requests.claim`. */
export function useClaimEventAce(eventId: number) {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (args: { req: string; body: ClaimAceRequest }): Promise<AceRequest> => {
      const { data, error } = await ois.POST("/api/v1/events/{id}/ace/{req}/claim", {
        params: { path: { id: eventId, req: args.req } },
        body: args.body,
      });
      if (error || !data) throw new Error("claim failed");
      return data;
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["event-ace", eventId] });
      toast.success("Slot claimed");
    },
    onError: () => toast.error("Couldn’t claim — someone may have beaten you to it"),
  });
}

/** Release your own claim on a request. Needs `ace.requests.claim`. */
export function useReleaseEventAce(eventId: number) {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (req: string): Promise<AceRequest> => {
      const { data, error } = await ois.DELETE("/api/v1/events/{id}/ace/{req}/claim", {
        params: { path: { id: eventId, req } },
      });
      if (error || !data) throw new Error("release failed");
      return data;
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["event-ace", eventId] });
      toast.success("Claim released");
    },
    onError: () => toast.error("Couldn’t release the claim"),
  });
}

export type Tier1Result = components["schemas"]["Tier1GenerateResult"];

/** Fan out ACE requests to the host ARTCC's Tier-1 neighbours (FNO events). Needs `ace.requests.create`. */
export function useGenerateTier1(eventId: number) {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (): Promise<Tier1Result> => {
      const { data, error } = await ois.POST("/api/v1/events/{id}/ace/tier1", {
        params: { path: { id: eventId } },
      });
      if (error || !data) throw new Error("generate failed");
      return data;
    },
    onSuccess: (r) => {
      qc.invalidateQueries({ queryKey: ["event-ace", eventId] });
      const made = r.created.length;
      const skipped = r.skipped.length;
      if (made === 0 && skipped === 0) {
        toast.success("No Tier-1 neighbours to request");
      } else {
        toast.success(
          `Created ${made} request${made === 1 ? "" : "s"}` +
            (skipped > 0 ? ` (skipped ${skipped} already open)` : ""),
        );
      }
    },
    onError: () => toast.error("Couldn’t generate Tier-1 requests"),
  });
}

/** Complete or cancel a request. Needs `ace.requests.decide`. */
export function useDecideEventAce(eventId: number) {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (args: { req: string; outcome: string }): Promise<AceRequest> => {
      const { data, error } = await ois.POST("/api/v1/events/{id}/ace/{req}/decide", {
        params: { path: { id: eventId, req: args.req } },
        body: { outcome: args.outcome },
      });
      if (error || !data) throw new Error("decide failed");
      return data;
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["event-ace", eventId] });
    },
    onError: () => toast.error("Couldn’t update the request"),
  });
}
