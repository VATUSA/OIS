import {useMutation, useQuery, useQueryClient} from "@tanstack/react-query";

import {ois} from "./api";

/**
 * Per-user preferences for a namespace — an opaque, client-owned jsonb blob stored server-side
 * (see backend migration 0034). The value shape is owned by the caller, so pass a type param;
 * the backend never inspects it. GET yields `null` when unset (the API returns `{}`).
 */
export function usePreferences<T>(namespace: string, options?: { enabled?: boolean }) {
  return useQuery({
    enabled: options?.enabled ?? true,
    queryKey: ["preferences", namespace],
    queryFn: async (): Promise<T | null> => {
      const { data, error } = await ois.GET("/api/v1/me/preferences/{namespace}", {
        params: { path: { namespace } },
      });
      if (error || data == null) return null;
      return data as T;
    },
    retry: false,
    staleTime: 60_000,
  });
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
