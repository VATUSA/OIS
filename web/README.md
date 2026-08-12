# web

The OIS website — **Vite + React** with TanStack Router (code-based routes) and TanStack
Query. shadcn/ui components + the light/dark theme live in the shared **`@ois/ui`**
package (so the Tauri desktop reuses them).

```bash
pnpm --filter web dev      # http://localhost:5173
pnpm --filter web build
```

## API types + client

Types come from the shared **`@ois/api-client`** package, generated from the backend's
OpenAPI (`openapi-typescript`) and wrapped with `openapi-fetch`:

```ts
// src/lib/api.ts
import { createOisClient } from "@ois/api-client";
export const ois = createOisClient(import.meta.env.VITE_OIS_API_URL ?? "http://127.0.0.1:3000");
```

Regenerate types (backend running): `pnpm --filter @ois/api-client codegen`. The web app
owns the app-specific glue: the `ThemeProvider` + `QueryClientProvider` mount, the
session-cookie auth (`credentials: "include"`), and `VITE_OIS_API_URL`.

## Auth flow

The **Sign in** button does a full-page navigation to
`GET /api/v1/auth/vatsim/login?return_to=<web origin>`; the backend runs VATSIM OAuth,
sets the session cookie, and redirects back; the app then calls `/api/v1/me`. (For local
dev, set `CORS_ALLOWED_ORIGINS=http://localhost:5173` on the backend.)

## Structure

- `src/lib/` — `api.ts` (client), `auth.ts` (`useMe`, `login`, `useLogout`)
- `src/components/` — app shell (nav bar)
- `src/pages/` — route pages
- `src/router.tsx` — TanStack Router tree
