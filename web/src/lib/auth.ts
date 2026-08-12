import {useMutation, useQuery, useQueryClient,} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {API_BASE, ois} from "./api";

export type Me = components["schemas"]["MeBody"];

async function fetchMe(): Promise<Me | null> {
  const { data, error } = await ois.GET("/api/v1/me");
  if (error || !data) return null;
  return data;
}

/** Current user, or `null` when signed out. Never throws on 401. */
export function useMe() {
  return useQuery({
    queryKey: ["me"],
    queryFn: fetchMe,
    retry: false,
    staleTime: 60_000,
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
