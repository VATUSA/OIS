import createClient from "openapi-fetch";

import type {paths} from "./generated/schema";

/**
 * Creates a typed OIS API client. The base URL + credentials are passed in so each app
 * (web, desktop) configures its own host; `credentials: "include"` sends the session
 * cookie on same-site requests.
 */
export function createOisClient(baseUrl: string) {
  return createClient<paths>({ baseUrl, credentials: "include" });
}

export type OisClient = ReturnType<typeof createOisClient>;
