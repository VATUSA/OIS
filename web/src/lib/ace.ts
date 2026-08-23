import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {useToast} from "@ois/ui";
import {ois} from "./api";

export type AceRequest = components["schemas"]["AceRequestBody"];
export type AceClaim = components["schemas"]["AceClaimBody"];
export type AceTeamMember = components["schemas"]["AceTeamMemberBody"];
export type CreateAceRequest = components["schemas"]["CreateAceRequestRequest"];
export type ClaimAceRequest = components["schemas"]["ClaimAceRequest"];
export type UpsertAceTeamMember = components["schemas"]["UpsertAceTeamMemberRequest"];

const TEAM = ["ace-team"] as const;

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

/** The ACE team roster. Needs `ace.team.read`. */
export function useAceTeam(all = false) {
  return useQuery({
    queryKey: [...TEAM, all],
    queryFn: async (): Promise<AceTeamMember[]> => {
      const { data, error } = await ois.GET("/api/v1/ace/team", {
        params: { query: all ? { all: true } : {} },
      });
      if (error || !data) throw new Error("failed to load ACE team");
      return data;
    },
  });
}

/** Add or update a roster member (by CID). Needs `ace.team.update`. */
export function useUpsertAceTeamMember() {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (body: UpsertAceTeamMember): Promise<AceTeamMember[]> => {
      const { data, error } = await ois.PUT("/api/v1/ace/team", { body });
      if (error || !data) throw new Error("save failed");
      return data;
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: TEAM });
      toast.success("Roster updated");
    },
    onError: () => toast.error("Couldn’t update the roster (is the CID a known OIS user?)"),
  });
}

/** Remove a roster member (by CID). Needs `ace.team.update`. */
export function useRemoveAceTeamMember() {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (cid: number): Promise<void> => {
      const { error } = await ois.DELETE("/api/v1/ace/team/{cid}", {
        params: { path: { cid } },
      });
      if (error) throw new Error("remove failed");
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: TEAM }),
    onError: () => toast.error("Couldn’t remove the member"),
  });
}
