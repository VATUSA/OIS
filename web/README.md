# web

The OIS website (Next.js). Scaffolded in Phase 3 with the app router.

## API types + client

API types are not hand-written. They come from the shared **`@ois/api-client`**
package, which generates them from the backend's OpenAPI with `openapi-typescript` and wraps them with `openapi-fetch` +
TanStack Query hooks (same tooling as osmium's website).
See [../packages/api-client/README.md](../packages/api-client/README.md).

```ts
// web/lib/ois.ts
import { createOisClient } from "@ois/api-client";
export const ois = createOisClient(process.env.NEXT_PUBLIC_OIS_API_URL!);
```

The web app owns only the app-specific glue: the `QueryProvider` mount, auth/session cookie handling, and base URL
(`NEXT_PUBLIC_OIS_API_URL`). Regenerate types (backend running) with `pnpm --filter @ois/api-client codegen`.

## Key surfaces

The permission editor (grouped-checkbox access UI), roster, events (approval workflow), TMU/NTML dashboards, the ACE
request queue, and flow dashboards. See
[../docs/PLAN.md](../docs/PLAN.md).
