import {API_BASE, ois} from "@/lib/api";
import {getDesktopToken, setDesktopToken} from "@/lib/desktop-token";
import {invokeDesktop, isTauri} from "@/lib/platform";

/**
 * Desktop sign-in, refresh and sign-out (#346).
 *
 * The desktop app authenticates with a session token held in the OS keychain rather than the web's
 * cookie, because a Tauri webview has no browser origin to carry one. The token is obtained through
 * the same VATSIM OAuth flow the website uses — opened in the **system browser**, so the app never
 * sees the user's VATSIM credentials — and the shell hands back a single-use code, which is traded
 * here for the real token.
 */

/**
 * Persists a freshly issued token to the in-memory cache and the keychain, in that order.
 *
 * Cache first: a refresh has already invalidated the previous token server-side, so any request
 * starting between here and the keychain write would otherwise send a token that is already dead.
 */
async function store(token: string) {
  setDesktopToken(token);
  await invokeDesktop<void>("store_token", {token});
}

/**
 * Signs in: runs the loopback OAuth flow in the system browser and stores the resulting token.
 *
 * `begin_login` blocks in the Rust shell until the browser redirects to its loopback listener, so
 * this promise is pending for as long as the user is typing their VATSIM password.
 */
export async function desktopLogin(): Promise<void> {
  if (!isTauri()) throw new Error("desktopLogin() is only available in the desktop app");

  // Tauri maps command args to camelCase; the shell takes the API base from here so the two
  // never disagree about which backend is being signed in to.
  const code = await invokeDesktop<string>("begin_login", {apiBase: API_BASE});
  const {data, error} = await ois.POST("/api/v1/auth/desktop/exchange", {body: {code}});
  if (error || !data) throw new Error("Sign-in failed: the code was rejected");

  await store(data.token);
}

/**
 * Rotates the stored token, returning when it next expires.
 *
 * Rotation is why this exists rather than a sliding expiry: the old token stops working the moment
 * this succeeds, so one that leaked has a bounded life. Returns `undefined` when there is nothing
 * to refresh or the token is no longer valid — the caller should treat that as signed out.
 */
export async function desktopRefresh(): Promise<string | undefined> {
  if (!isTauri()) return undefined;
  if (!(await getDesktopToken())) return undefined;

  const {data, error} = await ois.POST("/api/v1/auth/desktop/refresh", {});
  if (error || !data) {
    setDesktopToken(undefined);
    return undefined;
  }

  await store(data.token);
  return data.expires_at;
}

/**
 * Signs out: revokes the session server-side, then clears the keychain.
 *
 * The keychain is cleared even if the server call fails, so a user who asked to sign out is signed
 * out locally regardless — the token expires on its own in any case.
 */
export async function desktopLogout(): Promise<void> {
  if (!isTauri()) return;

  try {
    await ois.POST("/api/v1/auth/logout", {});
  } finally {
    setDesktopToken(undefined);
    await invokeDesktop<void>("delete_token").catch(() => undefined);
  }
}
