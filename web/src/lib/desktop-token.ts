import {invokeDesktop, isTauri} from "@/lib/platform";

/**
 * The desktop session token, cached in memory.
 *
 * Deliberately import-free apart from the platform seam: `lib/api.ts` reads this to attach the
 * Authorization header, and `lib/desktop-auth.ts` (which drives login/refresh) imports the API
 * client. Keeping the cache separate from the flow is what stops those two forming a cycle.
 *
 * The keychain is the source of truth and survives restarts; this only spares us a keychain round
 * trip on every request.
 */
let cached: string | undefined;
let loading: Promise<string | undefined> | undefined;

/** Replaces the cached token after a login or refresh, or clears it on logout. */
export function setDesktopToken(token: string | undefined) {
  cached = token;
  loading = undefined;
}

/**
 * The current desktop token, reading the OS keychain on first use.
 *
 * Returns `undefined` on the web build and when nobody is signed in, which the API client treats
 * the same way: send no Authorization header and let the cookie (or anonymity) decide.
 *
 * Concurrent callers share one in-flight keychain read — on a cold start every query fires at once,
 * and without this each would prompt its own read.
 */
export async function getDesktopToken(): Promise<string | undefined> {
  if (cached) return cached;
  if (!isTauri()) return undefined;

  loading ??= invokeDesktop<string | null>("get_token")
    .then((token) => {
      cached = token ?? undefined;
      return cached;
    })
    .catch(() => undefined)
    .finally(() => {
      loading = undefined;
    });

  return loading;
}
