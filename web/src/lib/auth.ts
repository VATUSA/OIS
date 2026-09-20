import {useMutation, useQuery, useQueryClient,} from "@tanstack/react-query";
import type {components} from "@ois/api-client";

import {API_BASE, ois} from "./api";
import {desktopLogin, desktopLogout} from "./desktop-auth";
import {isTauri} from "./platform";

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

/**
 * Starts sign-in.
 *
 * On the web that's a full-page redirect into the backend's VATSIM OAuth flow, returning here after.
 * The desktop app can't do that — navigating the webview away would lose the app, and it has no
 * origin to receive the session cookie — so it runs the flow in the system browser instead and
 * stores the resulting token in the keychain (#346). Awaiting the returned promise is optional; web
 * callers never get the chance, because the page is already navigating away.
 */
export async function login(): Promise<void> {
  if (isTauri()) {
    await desktopLogin();
    return;
  }

  const returnTo = `${window.location.origin}/`;
  window.location.href = `${API_BASE}/api/v1/auth/vatsim/login?return_to=${encodeURIComponent(
    returnTo,
  )}`;
}

export function useLogout() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async () => {
      // On desktop this also clears the keychain, so the next launch starts signed out.
      if (isTauri()) {
        await desktopLogout();
        return;
      }
      await ois.POST("/api/v1/auth/logout");
    },
    onSuccess: () => {
      queryClient.setQueryData(["me"], null);
    },
  });
}
