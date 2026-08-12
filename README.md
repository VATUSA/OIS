# OIS

VATUSA Event Operational Information System — a custom Rust backend, a Vite + React web app, a Rust Discord bot, and (later) a Tauri
desktop app. It brings VATUSA's event-operations, traffic-management (superseding
[vatflow.io](https://vatflow.io) — [source](https://github.com/djbrombizzle/vatflow)), ACE, and access tooling under one
platform, working alongside the current VATUSA website — which retains event creation, review, and posting.

The authorization model and service architecture are ported from
[osmium](../osmium) (the vZDC backend), evolved to national, multi-ARTCC scale.

## Layout

| Path                  | Stack           | What it is                                                                                                |
|-----------------------|-----------------|-----------------------------------------------------------------------------------------------------------|
| `backend/`            | Rust · Axum     | The API. Postgres, sqlx, embedded migrations, OpenAPI.                                                    |
| `discord/`            | Rust · serenity | The bot. Drains the backend outbound-job queue; calls back via REST as a service account.                 |
| `web/`                | Vite · React    | The website. TanStack Router + Query; consumes the generated typed API client.                            |
| `desktop/`            | Tauri           | Native app (Phase 5). Distinct feature set, not a website wrapper.                                        |
| `crates/ois-core`     | Rust            | Shared, DB-free domain + permission types (ported from osmium).                                           |
| `crates/ois-client`   | Rust            | Typed backend API client used by the bot.                                                                 |
| `packages/api-client` | TypeScript      | Shared OpenAPI-generated client (`openapi-typescript` + `openapi-fetch` + query hooks) for web + desktop. |
| `packages/*`          | TypeScript      | Shared UI, shared config.                                                                                 |
| `docs/`               | —               | Architecture, permissions, and per-feature specs.                                                         |

Two workspace managers coexist: a virtual **Cargo** workspace (`backend`, `discord`,
`crates/*`, later `desktop/src-tauri`) and a **pnpm + Turborepo** workspace (`web`,
`desktop`, `packages/*`). A root `justfile` ties cross-language tasks together.

## Getting started

```bash
cp .env.example .env
just up          # start Postgres
just backend     # run the API (migrations apply on startup)
just web         # run the web app
```

See [docs/PLAN.md](docs/PLAN.md) for the phased build order and current status.
