import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {useToast} from "@ois/ui";
import {ois} from "./api";

export type AceRequest = components["schemas"]["AceRequestBody"];
export type AceTeamMember = components["schemas"]["AceTeamMemberBody"];
export type CreateAceRequest = components["schemas"]["CreateAceRequestRequest"];
export type UpsertAceTeamMember = components["schemas"]["UpsertAceTeamMemberRequest"];

const REQUESTS = ["ace-requests"] as const;
const TEAM = ["ace-team"] as const;

/** The ACE support request queue (optionally filtered by status). Needs `ace.requests.read`. */
export function useAceRequests(status?: string) {
  return useQuery({
    queryKey: [...REQUESTS, status ?? "all"],
    queryFn: async (): Promise<AceRequest[]> => {
      const { data, error } = await ois.GET("/api/v1/ace/requests", {
        params: { query: status ? { status } : {} },
      });
      if (error || !data) throw new Error("failed to load ACE requests");
      return data;
    },
    refetchInterval: 30_000,
  });
}

/** Open an ACE support request. Needs `ace.requests.create`. */
export function useCreateAceRequest() {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (body: CreateAceRequest): Promise<AceRequest> => {
      const { data, error } = await ois.POST("/api/v1/ace/requests", { body });
      if (error || !data) throw new Error("create failed");
      return data;
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: REQUESTS });
      toast.success("ACE support requested");
    },
    onError: () => toast.error("Couldn’t submit the request"),
  });
}

/** Claim an open request. Needs `ace.requests.claim`. */
export function useClaimAceRequest() {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (id: string): Promise<void> => {
      const { error } = await ois.POST("/api/v1/ace/requests/{id}/claim", {
        params: { path: { id } },
      });
      if (error) throw new Error("claim failed");
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: REQUESTS }),
    onError: () => toast.error("Couldn’t claim — someone may have beaten you to it"),
  });
}

/** Complete or cancel a request. Needs `ace.requests.decide`. */
export function useDecideAceRequest() {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (args: { id: string; outcome: "completed" | "cancelled" }): Promise<void> => {
      const { error } = await ois.POST("/api/v1/ace/requests/{id}/decide", {
        params: { path: { id: args.id } },
        body: { outcome: args.outcome },
      });
      if (error) throw new Error("decide failed");
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: REQUESTS }),
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
