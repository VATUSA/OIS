import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {useToast} from "@ois/ui";
import {ois} from "./api";

export type Group = components["schemas"]["GroupBody"];
export type GroupMemberPage = components["schemas"]["GroupMemberPage"];
export type GroupMemberRequest = components["schemas"]["GroupMemberRequest"];
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

/** A page of a group's holders, each with the scope they hold it at. */
export function useGroupMembers(name: string, page = 1) {
  return useQuery({
    queryKey: [...GROUPS, name, "members", page],
    queryFn: async (): Promise<GroupMemberPage> => {
      const {data, error} = await ois.GET("/api/v1/admin/groups/{name}/members", {
        params: {path: {name}, query: {page}},
      });
      if (error || !data) throw new Error("failed to load members");
      return data;
    },
  });
}

/**
 * Add or remove one membership, at one scope.
 *
 * `artcc_id` matters on **removal** as well: a user can hold the same group nationally and at an
 * ARTCC, so omitting it would be ambiguous. The server rejects (403) a grant the caller could not
 * make directly.
 */
export function useChangeMembership(held: boolean) {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (args: {name: string; body: GroupMemberRequest}): Promise<void> => {
      const call = held ? ois.POST : ois.DELETE;
      // 204, so there is no body to read — the list is refetched by the invalidation below.
      const {error, response} = await call("/api/v1/admin/groups/{name}/members", {
        params: {path: {name: args.name}},
        body: args.body,
      });
      if (error) {
        const err = new Error("membership change failed") as Error & {status?: number};
        err.status = response?.status;
        throw err;
      }
    },
    onSuccess: () => qc.invalidateQueries({queryKey: GROUPS}),
    onError: (error: Error & {status?: number}) => {
      toast.error(
        error.status === 403
          ? "You can only grant a group where you hold everything it bundles"
          : held
            ? "Couldn’t add the member"
            : "Couldn’t remove the member",
      );
    },
  });
}

export type VatusaRoleMapping = components["schemas"]["VatusaRoleMappingBody"];
export type VatusaRoleMappingList = components["schemas"]["VatusaRoleMappingList"];
export type CreateVatusaRoleMappingRequest =
  components["schemas"]["CreateVatusaRoleMappingRequest"];

/** Under `GROUPS`, so invalidating the groups also refetches these (and vice versa). */
export const VATUSA_ROLE_MAPPINGS = [...GROUPS, "vatusa-role-mappings"] as const;

/** Every VATUSA role → group mapping, plus the VATUSA roles seen in synced members (#548). */
export function useVatusaRoleMappings() {
  return useQuery({
    queryKey: VATUSA_ROLE_MAPPINGS,
    queryFn: async (): Promise<VatusaRoleMappingList> => {
      const {data, error} = await ois.GET("/api/v1/admin/vatusa-role-mappings");
      if (error || !data) throw new Error("failed to load VATUSA role mappings");
      return data;
    },
  });
}

/**
 * Add or remove a mapping. Either changes access for every synced member holding that VATUSA role,
 * immediately, so the whole groups tree is invalidated — member lists move too. The server rejects
 * (403) a mapping to a group the caller couldn't grant directly.
 */
export function useAddVatusaRoleMapping() {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (body: CreateVatusaRoleMappingRequest): Promise<VatusaRoleMapping> => {
      const {data, error, response} = await ois.POST("/api/v1/admin/vatusa-role-mappings", {body});
      if (error || !data) {
        const err = new Error("mapping failed") as Error & {status?: number};
        err.status = response?.status;
        throw err;
      }
      return data;
    },
    onSuccess: () => qc.invalidateQueries({queryKey: GROUPS}),
    onError: (error: Error & {status?: number}) => {
      toast.error(
        error.status === 403
          ? "You can only map a group where you hold everything it bundles"
          : error.status === 409
            ? "That VATUSA role already grants this group"
            : "Couldn’t add the VATUSA role",
      );
    },
  });
}

export function useRemoveVatusaRoleMapping() {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (id: number): Promise<void> => {
      const {error} = await ois.DELETE("/api/v1/admin/vatusa-role-mappings/{id}", {
        params: {path: {id}},
      });
      if (error) throw new Error("mapping removal failed");
    },
    onSuccess: () => qc.invalidateQueries({queryKey: GROUPS}),
    onError: () => toast.error("Couldn’t remove the VATUSA role"),
  });
}
