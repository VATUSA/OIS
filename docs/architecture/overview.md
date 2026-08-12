# Architecture overview

This is the map. Deeper detail lives in the sibling docs:
[data-model.md](data-model.md) (schemas + tables),
[permissions.md](permissions.md) (the access model),
[api-conventions.md](api-conventions.md) (REST shape, auth, errors, OpenAPI), and
[integrations.md](integrations.md) (VATSIM, VATUSA, Discord, email).

## Services

- **backend** (`backend/`) — a single Axum (Rust 2024) binary over Postgres. Per-domain schemas (`identity`, `access`,
  `org`, `events`, `tmu`, `ace`, `flow`, `integration`,
  `media`, `stats`, `email`, `platform`, `web`). sqlx with migrations embedded in the binary and applied on startup.
  Versioned REST under `/api/v1`, self-served OpenAPI + docs. Background workers in `src/jobs`.
- **discord** (`discord/`) — a serenity/poise bot. Owns no data: it drains
  `integration.outbound_jobs` from the backend and calls back via REST as a service account for interactions (claim
  buttons, slash commands).
- **web** (`web/`) — Vite + React (TanStack Router + Query), consuming a typed client generated from the backend's OpenAPI (`packages/api-client`). shadcn/ui components live in the shared `packages/ui`.
- **desktop** (`desktop/`) — Tauri (Phase 5). Shares UI packages with web; native-only features (live flow monitor,
  always-on TMU display, notifications).

## Backend internal shape (ported from osmium)

```
backend/src/
  auth/       acl + RequirePermission + middleware (DB-backed; pure parts in ois-core)
  handlers/   thin HTTP handlers, one module per API domain
  repos/      SQL query layer — all SQL lives here, handlers stay thin
  models/     request/response + row types
  jobs/       background workers (roster sync, event lifecycle, outbound queue drain…)
  docs/       markdown + generated OpenAPI, served by the app
```

## Contract flow

Backend emits OpenAPI (utoipa) at `/docs/api/v1/openapi.json`. Types are generated from it into the shared **
`packages/api-client`** (`@ois/api-client`) — the same tooling as osmium's website, lifted into a package so `web` and
`desktop` share one client:

- **`openapi-typescript`** generates `packages/api-client/src/generated/schema.d.ts`
  (`paths` + `components`) via the package's `codegen` script, pointed at the running backend (`OIS_OPENAPI_URL`,
  default `http://127.0.0.1:3000/docs/api/v1/openapi.json`).
- **`openapi-fetch`** — `createOisClient(baseUrl)` wraps `createClient<paths>(...)`; the base URL + credentials are
  passed in so each app configures its own host.
- Per-domain **TanStack Query hooks** in `packages/api-client/src/hooks/*` over the typed client. The `QueryProvider`
  mount + auth glue stay in the consuming app.

The Discord bot does not use this — it uses the Rust `crates/ois-client` instead.

## Integration pattern (backend ↔ bot)

1. A backend action (event published, TMI issued, ACE request opened) inserts a row into
   `integration.outbound_jobs` with a `job_type` and JSON `payload`.
2. The bot polls the backend for pending jobs, executes the Discord side effect (create thread + ping, post embed), and
   acks the job (success/failure + attempt count).
3. When a user interacts with a bot message (claims an ACE request), the bot calls the backend API as a service account;
   the backend applies the state change and enqueues any follow-up jobs (e.g. notify the EC).

This keeps business logic in one place (the backend) and the bot stateless.
