import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {ois} from "./api";

export type PermTree = { [key: string]: PermTree | string[] };
export type UpdateBody = components["schemas"]["UpdateUserAccessRequest"];
export type UserMatch = components["schemas"]["UserSummary"];

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
    },
  });
}
