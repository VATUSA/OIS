import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {useToast} from "@ois/ui";
import {ois} from "./api";

export type ServiceAccount = components["schemas"]["ServiceAccountBody"];
export type ServiceAccountToken = components["schemas"]["ServiceAccountTokenBody"];
export type CreateServiceAccountRequest = components["schemas"]["CreateServiceAccountRequest"];
export type SetServiceAccountRolesRequest =
  components["schemas"]["SetServiceAccountRolesRequest"];
export type SetServiceAccountPermissionsRequest =
  components["schemas"]["SetServiceAccountPermissionsRequest"];

export const ACCOUNTS = ["service-accounts"] as const;
const ROLES = ["service-account-roles"] as const;
const GRANTABLE = ["service-account-grantable"] as const;

/** Credential lifetimes an admin may pick, in days. The backend defaults to 90 and caps at 365. */
export const EXPIRY_CHOICES = [30, 90, 180, 365] as const;
export const DEFAULT_EXPIRY_DAYS = 90;

/** Every service account, with its granted roles. */
export function useServiceAccounts() {
  return useQuery({
    queryKey: ACCOUNTS,
    queryFn: async (): Promise<ServiceAccount[]> => {
      const {data, error} = await ois.GET("/api/v1/admin/service-accounts");
      if (error || !data) throw new Error("failed to load service accounts");
      return data;
    },
  });
}

/**
 * The roles a service account may hold. Deliberately *not* the access catalog: that serves
 * ASSIGNABLE_USER_ROLES, the human editor's list, which omits the machine roles — so a picker
 * built from it could never grant BOT. This endpoint returns the same list the backend validates
 * against, so the two cannot disagree.
 */
export function useAssignableRoles() {
  return useQuery({
    queryKey: ROLES,
    queryFn: async (): Promise<string[]> => {
      const {data, error} = await ois.GET("/api/v1/admin/service-accounts/roles");
      if (error || !data) throw new Error("failed to load assignable roles");
      return data;
    },
    staleTime: 5 * 60_000,
  });
}

/**
 * What the signed-in admin may grant a service account, with the scope they hold it at. A grant
 * outside this is refused by the backend, so the picker never offers one.
 */
export function useGrantableServiceAccountPermissions() {
  return useQuery({
    queryKey: GRANTABLE,
    queryFn: async () => {
      const {data, error} = await ois.GET("/api/v1/admin/service-accounts/grantable-permissions");
      if (error || !data) throw new Error("failed to load grantable permissions");
      return data;
    },
    staleTime: 5 * 60_000,
  });
}

/**
 * Add a just-issued token to the on-screen reveals, newest first. A token is shown only once, so
 * another account's reveal is never dropped; the same account re-issued replaces its dead token.
 */
export function withReveal(
  reveals: readonly ServiceAccountToken[],
  token: ServiceAccountToken,
): ServiceAccountToken[] {
  return [token, ...reveals.filter((t) => t.account.id !== token.account.id)];
}

export function withoutReveal(
  reveals: readonly ServiceAccountToken[],
  accountId: string,
): ServiceAccountToken[] {
  return reveals.filter((t) => t.account.id !== accountId);
}

/** Create an account. The plaintext token in the result is shown once — never returned again. */
export function useCreateServiceAccount() {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (body: CreateServiceAccountRequest): Promise<ServiceAccountToken> => {
      const {data, error} = await ois.POST("/api/v1/admin/service-accounts", {body});
      if (error || !data) throw new Error("create failed");
      return data;
    },
    onSuccess: () => qc.invalidateQueries({queryKey: ACCOUNTS}),
    onError: () => toast.error("Couldn’t create the service account"),
  });
}

/** Replace an account's roles. A full replace, not a patch. */
export function useSetServiceAccountRoles() {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (args: {
      id: string;
      body: SetServiceAccountRolesRequest;
    }): Promise<ServiceAccount> => {
      const {data, error} = await ois.PUT("/api/v1/admin/service-accounts/{id}/roles", {
        params: {path: {id: args.id}},
        body: args.body,
      });
      if (error || !data) throw new Error("set roles failed");
      return data;
    },
    onSuccess: () => qc.invalidateQueries({queryKey: ACCOUNTS}),
    onError: () =>
      toast.error("Couldn’t update the roles", {
        description: "You can only grant a role whose permissions you hold nationally.",
      }),
  });
}

/** Replace an account's direct (permission, ARTCC) grants. A full replace, not a patch. */
export function useSetServiceAccountPermissions() {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (args: {
      id: string;
      body: SetServiceAccountPermissionsRequest;
    }): Promise<ServiceAccount> => {
      const {data, error} = await ois.PUT("/api/v1/admin/service-accounts/{id}/permissions", {
        params: {path: {id: args.id}},
        body: args.body,
      });
      if (error || !data) throw new Error("set permissions failed");
      return data;
    },
    onSuccess: () => qc.invalidateQueries({queryKey: ACCOUNTS}),
    onError: () => toast.error("Couldn’t update the permissions"),
  });
}

/** Revoke the live credential and issue a new one. The new token is shown once. */
export function useRotateServiceAccount() {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (args: {id: string; expiresInDays: number}): Promise<ServiceAccountToken> => {
      const {data, error} = await ois.POST("/api/v1/admin/service-accounts/{id}/rotate", {
        params: {path: {id: args.id}},
        body: {expires_in_days: args.expiresInDays},
      });
      if (error || !data) throw new Error("rotate failed");
      return data;
    },
    onSuccess: () => qc.invalidateQueries({queryKey: ACCOUNTS}),
    onError: () => toast.error("Couldn’t rotate the token"),
  });
}

/** Set or clear one account's rate limit (#611); `null` restores the deployment default. */
export function useSetServiceAccountRateLimit() {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async (args: {id: string; perMin: number | null}): Promise<void> => {
      const {error} = await ois.PUT("/api/v1/admin/service-accounts/{id}/rate-limit", {
        params: {path: {id: args.id}},
        body: {rate_limit_per_min: args.perMin},
      });
      if (error) throw new Error("set rate limit failed");
    },
    onSuccess: () => qc.invalidateQueries({queryKey: ACCOUNTS}),
    onError: () => toast.error("Couldn’t set the rate limit"),
  });
}
