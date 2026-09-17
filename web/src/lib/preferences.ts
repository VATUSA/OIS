import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";

import {ois} from "./api";

/**
 * Per-user preferences for a namespace — an opaque, client-owned jsonb blob stored server-side
 * (see backend migration 0034). The value shape is owned by the caller, so pass a type param;
 * the backend never inspects it. A failed load is a query error — never `null` — so callers that
 * write must wait for `isSuccess`, or they'd overwrite what's stored with a value built from nothing.
 */
export function usePreferences<T>(namespace: string) {
  return useQuery({
    queryKey: ["preferences", namespace],
    queryFn: async (): Promise<T | null> => {
      const { data, response } = await ois.GET("/api/v1/me/preferences/{namespace}", {
        params: { path: { namespace } },
      });
      return preferencesFrom<T>(response.ok, data);
    },
    retry: false,
    staleTime: 60_000,
  });
}

/**
 * A preferences GET's value: throws when the request failed, `null` when there's no body. Keyed on
 * `response.ok`, not `error` — openapi-fetch leaves `error` empty for a failure with no body.
 */
export function preferencesFrom<T>(ok: boolean, data: unknown): T | null {
  if (!ok) throw new Error("failed to load preferences");
  return (data ?? null) as T | null;
}

/** Upserts this user's preferences for a namespace and primes the query cache with the result. */
export function useSavePreferences<T>(namespace: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (value: T): Promise<T> => {
      const { error } = await ois.PUT("/api/v1/me/preferences/{namespace}", {
        params: { path: { namespace } },
        body: value,
      });
      if (error) throw new Error("save failed");
      return value;
    },
    onSuccess: (value) => {
      queryClient.setQueryData(["preferences", namespace], value);
    },
  });
}
