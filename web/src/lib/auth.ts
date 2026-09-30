import {useIsMutating, useMutation, useQuery, useQueryClient,} from "@tanstack/react-query";
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

/**
 * Sign-in as a mutation, so the UI actually reacts to it.
 *
 * On desktop `login()` resolves in place rather than navigating away, and `fetchMe` caches a 401 as
 * `null` ("signed out") for `staleTime`, so without invalidating `["me"]` the app keeps showing the
 * sign-in button for up to a minute after a successful sign-in. Failures matter too: the loopback
 * listener can fail to bind (port already held), time out, or have its code rejected — as a bare
 * `onClick={login}` those were a button that silently did nothing.
 */
export const LOGIN_MUTATION_KEY = ["auth", "login"] as const;

export function useLogin() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationKey: LOGIN_MUTATION_KEY,
    mutationFn: login,
    onSuccess: () => {
      void queryClient.invalidateQueries({queryKey: ["me"]});
    },
  });
}

/**
 * Whether *any* sign-in is in flight, anywhere in the app.
 *
 * A mutation's own `isPending` is per hook instance, and more than one sign-in button can be on
 * screen at once — signed out on a shared dashboard, the sidebar's identity button and the page's
 * call to action are both mounted. Clicking each in turn started two `begin_login` calls; the second
 * takes the loopback port from the first, so finishing the first browser tab then hands its code to a
 * listener expecting a different nonce, which rejects it and waits out the five-minute timeout in
 * silence (#428 review).
 *
 * Keyed off the shared {@link LOGIN_MUTATION_KEY} so every surface disables together.
 */
export function useSignInPending(): boolean {
  return useIsMutating({mutationKey: LOGIN_MUTATION_KEY}) > 0;
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
