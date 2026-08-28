import {useMutation, useQuery, useQueryClient,} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {API_BASE, ois} from "./api";

export type Me = components["schemas"]["MeBody"];

async function fetchMe(): Promise<Me | null> {
  // A reachable backend answers with the user (200) or a 401 body → treated as signed out (null).
  // A *network* failure (backend down / unreachable) makes `ois.GET` reject, which propagates as a
  // query error — the app uses that to show an "unreachable, retrying" state instead of hanging on
  // an infinite "Loading…" (every data-gated page would otherwise wait forever).
  const { data, error } = await ois.GET("/api/v1/me");
  if (error || !data) return null;
  return data;
}

/** Current user, or `null` when signed out. Errors only when the backend is unreachable. */
export function useMe() {
  return useQuery({
    queryKey: ["me"],
    queryFn: fetchMe,
    // Ride out a transient blip before surfacing an error…
    retry: 2,
    staleTime: 60_000,
    // …and once unreachable, keep probing so the app recovers on its own when the backend returns.
    refetchInterval: (query) =>
      query.state.status === "error" ? 5_000 : false,
  });
}

/** Full-page redirect into the backend's VATSIM OAuth flow, returning here after. */
export function login() {
  const returnTo = `${window.location.origin}/`;
  window.location.href = `${API_BASE}/api/v1/auth/vatsim/login?return_to=${encodeURIComponent(
    returnTo,
  )}`;
}

export function useLogout() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async () => {
      await ois.POST("/api/v1/auth/logout");
    },
    onSuccess: () => {
      queryClient.setQueryData(["me"], null);
    },
  });
}
