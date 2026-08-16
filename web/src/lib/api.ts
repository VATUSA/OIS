import {createOisClient} from "@ois/api-client";

declare global {
  interface Window {
    /** Injected at container start from OIS_API_URL (see deploy/40-ois-config.sh). */
    __OIS_API_URL__?: string;
    /** Injected at container start from DOCS_URL (see deploy/40-ois-config.sh). */
    __OIS_DOCS_URL__?: string;
  }
}

// API base, resolved in order:
//   1. window.__OIS_API_URL__ — set at container start from OIS_API_URL, so one built image
//      works in any environment (may be "" for a same-origin deployment).
//   2. VITE_OIS_API_URL — build-time env, for local dev.
//   3. localhost default.
const runtime =
  typeof window !== "undefined" ? window.__OIS_API_URL__ : undefined;
export const API_BASE =
  typeof runtime === "string"
    ? runtime
    : (import.meta.env.VITE_OIS_API_URL ?? "http://127.0.0.1:3000");

/** The shared, typed OIS API client (session cookie sent via credentials: include). */
export const ois = createOisClient(API_BASE);

// Docs site URL (external, its own subdomain per environment), resolved like API_BASE:
//   window.__OIS_DOCS_URL__ (container start) → VITE_OIS_DOCS_URL (dev) → "" (no link shown).
const docsRuntime =
  typeof window !== "undefined" ? window.__OIS_DOCS_URL__ : undefined;
export const DOCS_URL =
  (typeof docsRuntime === "string" ? docsRuntime : import.meta.env.VITE_OIS_DOCS_URL) ??
  "";
