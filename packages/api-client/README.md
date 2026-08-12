# @ois/api-client

Shared, OpenAPI-generated client for the OIS backend — consumed by both `web` and
`desktop`. Same tooling as osmium's website (`openapi-typescript` + `openapi-fetch` + TanStack Query), lifted into a
package so the generated types and hooks aren't duplicated per app.

## Layout (Phase 3)

```
src/
  generated/schema.d.ts   # openapi-typescript output (paths + components). Do not edit.
  client.ts               # createOisClient(baseUrl) -> createClient<paths>(...)
  hooks/                  # per-domain TanStack Query hooks over the typed client
  index.ts
```

## Regenerating types

The backend must be running (types come from its live `openapi.json`):

```bash
pnpm --filter @ois/api-client codegen
# override host: OIS_OPENAPI_URL=https://api.example/docs/api/v1/openapi.json pnpm --filter @ois/api-client codegen
```

## Usage

```ts
import {createOisClient} from "@ois/api-client";

export const ois = createOisClient(process.env.NEXT_PUBLIC_OIS_API_URL!);
```

The client factory takes the base URL so each app (web, desktop) configures its own host + credentials. Auth glue
(session cookie vs. token) and the `QueryProvider` mount stay in the consuming app.
