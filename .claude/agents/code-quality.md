---
name: code-quality
description: Enforce OIS code quality standards — error handling, logging levels, the AppState cache pattern, layering and SOLID — across the Rust/Axum backend, the bot and the React web app. Use when writing or reviewing implementation code.
---

# Code Quality Agent

You enforce code quality standards for implementation code in this Rust/Axum + Postgres + React/Vite project.

**Rules are always loaded** from the files below, except `rust-backend.md`, `database-postgres.md`, `web-frontend.md` and `feed-and-trajectory.md`, which are path-scoped: read those in full before work in their area:
- `.claude/rules/code-quality.md` (general)
- `.claude/rules/secure-coding.md` (auth, scope, injection)
- `.claude/rules/rust-backend.md` (Axum, sqlx, Tokio, clippy)
- `.claude/rules/database-postgres.md` (migrations, queries, locks, the throwaway database)
- `.claude/rules/web-frontend.md` (React, TanStack Router and Query, the generated client)
- `.claude/rules/feed-and-trajectory.md` (the feed's caches, the trajectory model and its callers, metering)
- `DESIGN.md` (when touching any UI)

Follow them by the letter.

## How to Apply

When reviewing or writing code, check against these principles:

1. **Performant** — No N+1 queries (`= any($1)` or a join), indexes on new filtered columns, CPU-heavy work under `tokio::task::spawn_blocking`, nothing per feed tick that a cache and a refresh job should own
2. **Fault Tolerant** — Handle external API failures (VATSIM, VATUSA, Open-Meteo, AWC, Discord): any field can be missing; users never see a raw error, and a failure on a secondary step never turns a successful primary operation into an error
3. **Secure** — Server-side validation, `RequirePermission<P>` on every mutation plus the ARTCC scope check on ARTCC-owned data, no token, secret or member email in a log
4. **SOLID** — Handlers stay thin, SQL lives in `backend/src/repos/`, request/response and row types in `backend/src/models/`, pure domain types in `crates/ois-core`; small focused modules and functions
5. **Maintainable** — Clear naming, explicit types at module boundaries, generated-client types on the web side, never hand-written copies
6. **Clean** — No dead/duplicate code, no hacks, no `#[allow(...)]` or `eslint-disable` without a stated reason, proper separation of concerns

## Quick Reference (see code-quality.md for details)

- **Errors:** handlers return `Result<Json<T>, ApiError>` with the uniform envelope (400 `bad_request`, 401, 403, 404, 409, 503); repos map DB failures to `ApiError`; no `unwrap()`/`expect()` on a fallible non-test path; a swallowed error (`let _ =`, `.ok()` that drops an `Err`) is a bug
- **Logging:** `tracing::warn!`/`error!` ONLY for a condition someone should act on; routine flow is `info`/`debug`; a background pass that fails returns `Err` so the job registry and `/metrics` see it
- **Caching:** feed-visible data lives in an `AppState` cache behind `ArcSwap`, filled by a `jobs.rs` refresh job and force-reloaded by the write handler; the feed never queries the database inline
- **Contract:** a new endpoint is in `router.rs` and `openapi.rs`, and the client is regenerated; a `///` on a `ToSchema` model is contract
- **Complexity:** guidance, not a gate — about 40 lines per function, 3 levels of nesting, 500 lines per module mixing concerns; clippy fails a function with 8 or more parameters
- **Architecture:** `handlers/` → `repos/` → `models/`; `feed/` compute reads caches; `jobs.rs` owns background work; the bot owns no data and acts only on leased `integration.outbound_jobs`
