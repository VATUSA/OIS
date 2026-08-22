import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {useToast} from "@ois/ui";
import {ois} from "./api";

export type ApiKey = components["schemas"]["ApiKeyBody"];
export type ApiKeyToken = components["schemas"]["ApiKeyTokenBody"];
export type ApiKeyPermission = components["schemas"]["ApiKeyPermissionBody"];
export type ApiKeyPermissionInput = components["schemas"]["ApiKeyPermissionInput"];
export type CreateApiKeyRequest = components["schemas"]["CreateApiKeyRequest"];
export type SetApiKeyPermissionsRequest = components["schemas"]["SetApiKeyPermissionsRequest"];
export type GrantablePermission = components["schemas"]["GrantablePermissionBody"];
export type AuditLogPage = components["schemas"]["AuditLogPage"];

const MINE = ["api-keys"] as const;
const ALL = ["admin-api-keys"] as const;

// --- self-service (your own keys) ---

/** Your API keys. */
export function useMyKeys() {
  return useQuery({
    queryKey: MINE,
    queryFn: async (): Promise<ApiKey[]> => {
      const { data, error } = await ois.GET("/api/v1/api-keys");
      if (error || !data) throw new Error("failed to load API keys");
      return data;
    },
  });
}

/** The permissions you can delegate to a key, each with the scope you can grant it at. */
export function useGrantablePermissions() {
  return useQuery({
    queryKey: [...MINE, "grantable"],
    queryFn: async (): Promise<GrantablePermission[]> => {
      const { data, error } = await ois.GET("/api/v1/api-keys/grantable-permissions");
      if (error || !data) throw new Error("failed to load grantable permissions");
      return data;
    },
  });
}

/** Create a key. The plaintext token in the result is shown once — never returned again. */
export function useCreateKey() {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (body: CreateApiKeyRequest): Promise<ApiKeyToken> => {
      const { data, error } = await ois.POST("/api/v1/api-keys", { body });
      if (error || !data) throw new Error("create failed");
      return data;
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: MINE }),
    onError: () => toast.error("Couldn’t create the API key"),
  });
}

/** Rotate a key's secret. Returns the new token once. */
export function useRotateKey() {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (id: string): Promise<ApiKeyToken> => {
      const { data, error } = await ois.POST("/api/v1/api-keys/{id}/rotate", {
        params: { path: { id } },
      });
      if (error || !data) throw new Error("rotate failed");
      return data;
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: MINE }),
    onError: () => toast.error("Couldn’t rotate the key"),
  });
}

/** Replace a key's granted permissions (re-validated against your live access). */
export function useSetKeyPermissions() {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (args: {
      id: string;
      body: SetApiKeyPermissionsRequest;
    }): Promise<ApiKey> => {
      const { data, error } = await ois.PUT("/api/v1/api-keys/{id}/permissions", {
        params: { path: { id: args.id } },
        body: args.body,
      });
      if (error || !data) throw new Error("update failed");
      return data;
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: MINE }),
    onError: () => toast.error("Couldn’t update permissions"),
  });
}

/** Disable a key without deleting it. */
export function useDisableKey() {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (id: string): Promise<void> => {
      const { error } = await ois.POST("/api/v1/api-keys/{id}/disable", {
        params: { path: { id } },
      });
      if (error) throw new Error("disable failed");
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: MINE }),
    onError: () => toast.error("Couldn’t disable the key"),
  });
}

/** Permanently delete a key. */
export function useDeleteKey() {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (id: string): Promise<void> => {
      const { error } = await ois.DELETE("/api/v1/api-keys/{id}", {
        params: { path: { id } },
      });
      if (error) throw new Error("delete failed");
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: MINE }),
    onError: () => toast.error("Couldn’t delete the key"),
  });
}

/** A key's activity dossier (owner, or an oversight admin). */
export function useKeyAudit(id: string | null, page: number) {
  return useQuery({
    queryKey: [...MINE, id, "audit", page],
    enabled: !!id,
    queryFn: async (): Promise<AuditLogPage> => {
      const { data, error } = await ois.GET("/api/v1/api-keys/{id}/audit", {
        params: { path: { id: id! }, query: { page } },
      });
      if (error || !data) throw new Error("failed to load key activity");
      return data;
    },
  });
}

// --- admin oversight (any user's keys) ---

/** All keys (admin), optionally filtered to one owner's CID. */
export function useAllKeys(ownerCid?: number) {
  return useQuery({
    queryKey: [...ALL, ownerCid ?? null],
    queryFn: async (): Promise<ApiKey[]> => {
      const { data, error } = await ois.GET("/api/v1/admin/api-keys", {
        params: { query: ownerCid ? { owner_cid: ownerCid } : {} },
      });
      if (error || !data) throw new Error("failed to load API keys");
      return data;
    },
  });
}

/** Admin: disable any user's key (optional reason recorded to the audit log). */
export function useAdminDisableKey() {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (args: { id: string; reason?: string }): Promise<void> => {
      const { error } = await ois.POST("/api/v1/admin/api-keys/{id}/disable", {
        params: { path: { id: args.id } },
        body: { reason: args.reason },
      });
      if (error) throw new Error("disable failed");
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ALL }),
    onError: () => toast.error("Couldn’t disable the key"),
  });
}

/** Admin: permanently delete any user's key. */
export function useAdminDeleteKey() {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (id: string): Promise<void> => {
      const { error } = await ois.DELETE("/api/v1/admin/api-keys/{id}", {
        params: { path: { id } },
      });
      if (error) throw new Error("delete failed");
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ALL }),
    onError: () => toast.error("Couldn’t delete the key"),
  });
}
