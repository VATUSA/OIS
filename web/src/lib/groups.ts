import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {useToast} from "@ois/ui";
import {ois} from "./api";

export type Group = components["schemas"]["GroupBody"];
export type CreateGroupRequest = components["schemas"]["CreateGroupRequest"];
export type UpdateGroupRequest = components["schemas"]["UpdateGroupRequest"];

export const GROUPS = ["access-groups"] as const;

/** Every group with what it grants and how many principals hold it. */
export function useGroups() {
  return useQuery({
    queryKey: GROUPS,
    queryFn: async (): Promise<Group[]> => {
      const {data, error} = await ois.GET("/api/v1/admin/groups");
      if (error || !data) throw new Error("failed to load groups");
      return data;
    },
  });
}

/**
 * Replace a group's permission set.
 *
 * One write changes every holder's access — there is no per-user backfill, which is the point of
 * groups. The server rejects (403) any permission the actor does not hold themselves.
 */
export function useSaveGroup() {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (args: {name: string; body: UpdateGroupRequest}): Promise<Group> => {
      const {data, error, response} = await ois.PUT("/api/v1/admin/groups/{name}", {
        params: {path: {name: args.name}},
        body: args.body,
      });
      if (error || !data) {
        const err = new Error("save failed") as Error & {status?: number};
        err.status = response?.status;
        throw err;
      }
      return data;
    },
    onSuccess: () => qc.invalidateQueries({queryKey: GROUPS}),
    onError: (error: Error & {status?: number}) => {
      toast.error(
        error.status === 403
          ? "You can only grant permissions you hold yourself"
          : "Couldn’t save the group",
      );
    },
  });
}

export function useCreateGroup() {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (body: CreateGroupRequest): Promise<Group> => {
      const {data, error, response} = await ois.POST("/api/v1/admin/groups", {body});
      if (error || !data) {
        const err = new Error("create failed") as Error & {status?: number};
        err.status = response?.status;
        throw err;
      }
      return data;
    },
    onSuccess: () => qc.invalidateQueries({queryKey: GROUPS}),
    onError: (error: Error & {status?: number}) => {
      toast.error(error.status === 409 ? "That group already exists" : "Couldn’t create the group");
    },
  });
}

/** Delete a group. Refused (409) while anyone still holds it — empty it first. */
export function useDeleteGroup() {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (name: string): Promise<void> => {
      const {error, response} = await ois.DELETE("/api/v1/admin/groups/{name}", {
        params: {path: {name}},
      });
      if (error) {
        const err = new Error("delete failed") as Error & {status?: number};
        err.status = response?.status;
        throw err;
      }
    },
    onSuccess: () => qc.invalidateQueries({queryKey: GROUPS}),
    onError: (error: Error & {status?: number}) => {
      toast.error(
        error.status === 409
          ? "That group still has members — remove them first"
          : "Couldn’t delete the group",
      );
    },
  });
}
