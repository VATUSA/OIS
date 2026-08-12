import {createOisClient} from "@ois/api-client";

export const API_BASE =
  import.meta.env.VITE_OIS_API_URL ?? "http://127.0.0.1:3000";

/** The shared, typed OIS API client (session cookie sent via credentials: include). */
export const ois = createOisClient(API_BASE);
