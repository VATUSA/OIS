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

- `src/lib/` — `api.ts` (client), `auth.ts` (`useMe`, `login`, `useLogout`),
  `platform.ts` (the desktop seam, below)
- `src/components/` — app shell (nav bar), `desktop-only.tsx`
- `src/pages/` — route pages
- `src/router.tsx` — TanStack Router tree

## Desktop (Tauri) — the platform seam

The desktop app renders **this same bundle** inside a Tauri webview; there is no second frontend.
Everything that differs between web and desktop goes through `src/lib/platform.ts`, so the two never
fork.

```tsx
import {can, invokeDesktop} from "@/lib/platform";
import {DesktopOnly} from "@/components/desktop-only";

// Gate on a specific ability — false on web, and false on desktop until that
// feature ships, so this is safe to write before the feature exists.
if (can("tray")) { ... }

// Gate UI on the platform itself. `fallback` is optional.
<DesktopOnly fallback={<span>Available in the desktop app</span>}>
  <PopOutButton />
</DesktopOnly>
```

### Calling a Tauri command

`invokeDesktop` is the only way the web app talks to the Rust shell. It pairs with a
`#[tauri::command]` in `desktop/src-tauri`, registered on the builder there:

```ts
const token = await invokeDesktop<string>("get_token", {cid});
```

```rust
#[tauri::command]
fn get_token(cid: u32) -> Result<String, String> { ... }
```

On the web build it throws (naming the command) rather than silently returning nothing, so an
ungated caller shows up immediately instead of failing quietly.

**One rule:** `@tauri-apps/api` must only ever be reached through a dynamic `import()` — which is
what `invokeDesktop` does. A static `import` at the top of any module pulls it into the main web
bundle. Detection itself (`isTauri()`) is just an `in` check against the global Tauri injects, so it
costs the web build nothing.

Capabilities are declared in `platform.ts` ahead of the features that implement them (#348–#354);
each is flipped on by the issue that builds it.
