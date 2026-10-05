import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";
import {useToast} from "@ois/ui";

import {ois} from "./api";

export type PermTree = { [key: string]: PermTree | string[] };
export type UpdateBody = components["schemas"]["UpdateUserAccessRequest"];
export type UserMatch = components["schemas"]["UserSummary"];
export type AdminUserRow = components["schemas"]["AdminUserRow"];

/**
 * One `scoped_roles` entry — `EC` for a national membership, `EC:ZDC` for one at a facility — split
 * into its parts. Group names are uppercase letters and underscores, so the first `:` is the split.
 */
export function splitScopedRole(entry: string): {role: string; artcc: string | null} {
  const at = entry.indexOf(":");
  return at < 0 ? {role: entry, artcc: null} : {role: entry.slice(0, at), artcc: entry.slice(at + 1)};
}

/** A page of all OIS users (name/CID/rating + role names), filtered by `q`. Access-admin only. */
export function useAllUsers(page: number, pageSize: number, q: string) {
  return useQuery({
    queryKey: ["admin-users", page, pageSize, q],
    placeholderData: (prev) => prev,
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/admin/users", {
        params: { query: { q, page, page_size: pageSize } },
      });
      if (error || !data) throw new Error("failed to load users");
      return data;
    },
  });
}

/** Fuzzy user search by name or CID (enabled once the term is non-empty). */
export function useUserSearch(term: string) {
  return useQuery({
    enabled: term.length >= 1,
    queryKey: ["user-search", term],
    placeholderData: (prev) => prev,
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/users", {
        params: { query: { q: term, limit: 12 } },
      });
      if (error || !data) throw new Error("search failed");
      return data;
    },
  });
}

/** Flatten a permission tree into concrete `segments.action` strings. */
export function flattenTree(tree: PermTree, prefix: string[] = []): string[] {
  const out: string[] = [];
  for (const [key, value] of Object.entries(tree ?? {})) {
    if (Array.isArray(value)) {
      for (const action of value) out.push([...prefix, key, action].join("."));
    } else if (value && typeof value === "object") {
      out.push(...flattenTree(value, [...prefix, key]));
    }
  }
  return out;
}

/** Build the nested permission tree the API expects from concrete strings. */
export function buildTree(names: Iterable<string>): PermTree {
  const root: PermTree = {};
  for (const name of names) {
    const parts = name.split(".");
    const action = parts.pop()!;
    let node = root;
    parts.forEach((segment, index) => {
      if (index === parts.length - 1) {
        const current = node[segment];
        if (Array.isArray(current)) {
          if (!current.includes(action)) current.push(action);
        } else {
          node[segment] = [action];
        }
      } else {
        if (!node[segment] || Array.isArray(node[segment])) node[segment] = {};
        node = node[segment] as PermTree;
      }
    });
  }
  return root;
}

export function useCatalog() {
  return useQuery({
    queryKey: ["access-catalog"],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/access/catalog");
      if (error || !data) throw new Error("failed to load catalog");
      return data;
    },
  });
}

export function useUserAccess(cid: number | undefined) {
  return useQuery({
    enabled: cid != null,
    retry: false,
    queryKey: ["user-access", cid],
    queryFn: async () => {
      const { data, error, response } = await ois.GET(
        "/api/v1/admin/users/{cid}/access",
        { params: { path: { cid: cid! } } },
      );
      if (error || !data) {
        const err = new Error("failed to load access") as Error & {
          status?: number;
        };
        err.status = response?.status;
        throw err;
      }
      return data;
    },
  });
}

export function useSaveUserAccess() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async ({ cid, body }: { cid: number; body: UpdateBody }) => {
      const { data, error, response } = await ois.POST(
        "/api/v1/admin/users/{cid}/access",
        { params: { path: { cid } }, body },
      );
      if (error || !data) {
        const err = new Error("save failed") as Error & { status?: number };
        err.status = response?.status;
        throw err;
      }
      return data;
    },
    onSuccess: (data, variables) => {
      queryClient.setQueryData(["user-access", variables.cid], data);
      // A save takes the user off VATUSA role sync (#549): refresh that state and the user list's flag.
      void queryClient.invalidateQueries({ queryKey: ["user-vatusa", variables.cid] });
      void queryClient.invalidateQueries({ queryKey: ["admin-users"] });
      toast.success("Access saved", { description: `CID ${variables.cid}` });
    },
    onError: () => toast.error("Couldn’t save access changes"),
  });
}

/** A user's VATUSA side for the access editor: role-sync state, VATUSA roles, and what a Resync would change (#549). */
export function useUserVatusa(cid: number | undefined) {
  return useQuery({
    enabled: cid != null,
    retry: false,
    queryKey: ["user-vatusa", cid],
    queryFn: async () => {
      const { data, error } = await ois.GET("/api/v1/admin/users/{cid}/vatusa", {
        params: { path: { cid: cid! } },
      });
      if (error || !data) throw new Error("failed to load VATUSA state");
      return data;
    },
  });
}

/** Put a hand-managed user back on VATUSA role sync and reconcile them now (#549). */
export function useVatusaResync() {
  const queryClient = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async ({ cid, reason }: { cid: number; reason: string }) => {
      const { data, error, response } = await ois.POST("/api/v1/admin/users/{cid}/vatusa/resync", {
        params: { path: { cid } },
        body: { reason },
      });
      if (error || !data) {
        const err = new Error("resync failed") as Error & { status?: number };
        err.status = response?.status;
        throw err;
      }
      return data;
    },
    onSuccess: (data, { cid }) => {
      queryClient.setQueryData(["user-vatusa", cid], data);
      void queryClient.invalidateQueries({ queryKey: ["user-access", cid] });
      void queryClient.invalidateQueries({ queryKey: ["admin-users"] });
      toast.success("Back on VATUSA sync", { description: `CID ${cid}` });
    },
    onError: (err: Error & { status?: number }) =>
      toast.error(
        err.status === 403
          ? "Resync needs national access.users.update"
          : "Couldn’t resync from VATUSA",
      ),
  });
}
