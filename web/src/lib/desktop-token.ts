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
// Bumped by every `setDesktopToken`. A keychain read begun before a logout resolves after it, and
// must not write the token it read back into the cache — that would restore a revoked session
// (VATUSA/OIS#346 review). A read only lands if nothing has replaced the token since it started.
let generation = 0;
// A read has completed — including one that found nothing, and one that failed.
//
// Without this a dismissed keychain prompt left no trace: `cached` stayed undefined and `loading`
// was cleared, so the very next request re-invoked `get_token` and prompted again. The token is
// resolved per API request, and `useMe` retries every 5s on error, so one dismissed prompt became a
// prompt every few seconds (VATUSA/OIS#535). `setDesktopToken` clears this, which is the recovery
// path: a failed read lands the user on the sign-in screen, and signing in sets the token.
let settled = false;

/** Replaces the cached token after a login or refresh, or clears it on logout. */
export function setDesktopToken(token: string | undefined) {
  generation += 1;
  cached = token;
  loading = undefined;
  settled = false;
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
  // A read already came back empty or failed. Reading again would prompt again, and nothing but
  // `setDesktopToken` can change the answer.
  if (settled) return undefined;

  if (loading) return loading;
  const started = generation;
  const read: Promise<string | undefined> = invokeDesktop<string | null>("get_token")
    .then((token) => {
      if (started !== generation) return cached; // superseded: report what is current, not what we read
      settled = true;
      cached = token ?? undefined;
      return cached;
    })
    .catch(() => {
      // The line that closes the loop: a dismissed or failing read is a result too.
      if (started === generation) settled = true;
      return undefined;
    })
    .finally(() => {
      if (loading === read) loading = undefined;
    });
  loading = read;
  return read;
}
