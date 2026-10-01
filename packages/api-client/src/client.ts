import createClient from "openapi-fetch";

import type {paths} from "./generated/schema";

/** Supplies the bearer token for a platform that can't use the session cookie (desktop). */
export type TokenProvider = () => string | undefined | Promise<string | undefined>;

/**
 * Creates a typed OIS API client. The base URL + credentials are passed in so each app
 * (web, desktop) configures its own host; `credentials: "include"` sends the session
 * cookie on same-site requests.
 *
 * `getToken` is how the desktop app authenticates instead: it has no browser origin and so cannot
 * carry the cookie, and sends `Authorization: Bearer ois_dsk_…` from the OS keychain (#346). Omit
 * it — as the web app does — and the client behaves exactly as it did before: no middleware is
 * registered and the cookie is the only credential.
 */
export function createOisClient(baseUrl: string, getToken?: TokenProvider) {
  const client = createClient<paths>({ baseUrl, credentials: "include" });

  if (getToken) {
    client.use({
      async onRequest({ request }) {
        // Resolved per request rather than captured once, so a refresh (or a logout) takes effect
        // on the very next call without rebuilding the client.
        const token = await getToken();
        if (token) request.headers.set("Authorization", `Bearer ${token}`);
        return request;
      },
    });
  }

  return client;
}

export type OisClient = ReturnType<typeof createOisClient>;
