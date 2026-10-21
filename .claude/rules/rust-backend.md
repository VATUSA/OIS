---
paths:
  - "backend/**"
  - "crates/**"
  - "discord/**"
  - "desktop/src-tauri/**"
  - "Cargo.toml"
  - "Cargo.lock"
---
# Rust Backend Rules (not loaded at startup; read in full before writing or reviewing Rust)

The architecture (handler → repo → model, the permission markers and their three-in-sync rule, the
API contract, the trajectory model and its callers, the feed's no-DB rule, realtime, auditing) is
`AGENTS.md` § Architecture; the error envelope and auth model are § Conventions & gotchas. Code
standards are `code-quality.md`, security is `secure-coding.md`, SQL is `database-postgres.md`, tests
are `test-quality.md` and `test-writing.md`. This file is the Rust and Axum layer on top.

---

## Clippy is the gate

`cargo clippy --workspace --all-targets -- -D warnings` runs in CI and in the full local gate
(`just test`); run it on every Rust change before calling it done. `--all-targets` matters: test
modules are compiled only by it. Two traps that have passed a narrower check and failed CI:

- **`items_after_test_module`.** New code appended below an existing `#[cfg(test)] mod tests` fails
  the lib-test build (#457). Put implementation above the test module.
- **`too_many_arguments`.** An eighth parameter fails the lint (#508). Group related parameters into a
  struct rather than reaching for `#[allow]`; it also stops call sites swapping same-typed arguments.

**Edit Rust source with the Edit tool**, never a non-raw Python string fed through a heredoc: Python
consumes the `\` of a Rust string's line continuation and leaves a run of spaces in the literal (#870).

## Running the Rust tests
- Targeted: `cargo nextest run -p ois-backend -E 'test(<name>)'`. nextest runs each test in its own
  process, which the `#[sqlx::test]` harness needs; many threads in one `cargo test` process collide on
  its per-test databases.
- Full: `just test` (`bin/run-tests.sh`), once per committed SHA (`test-quality.md` → "A gate counts
  once per SHA").
- `#[sqlx::test]` needs `DATABASE_URL` pointing at a running Postgres (`just up`); it creates its own
  database per test.

## Handlers
- Signature order: `State`, `RequirePermission<P>`, `Actor(principal)`, `Path`, `Query`, then the
  `Json` body last (it consumes the request). The permission and scope rules are `secure-coding.md`.
- Return `Result<Json<T>, ApiError>`. A conflict the caller can act on is
  `ApiError::ConflictReason("<stable_code>")`; a failed `If-Match` is `PreconditionFailed` (#585).
  Never invent a status outside `errors.rs`.
- **Register both halves**: the route in `router.rs` and the path (plus any new schema) in
  `openapi.rs`, then regenerate the client ("The contract" below).
- **Heavy CPU off the async workers.** Route resolution, metering and projection run under
  `tokio::task::spawn_blocking` (`metered_flights` in `handlers/flow.rs`), after taking `load_full()`
  clones of the caches they need and dropping the feed lock.
- **To test a handler's logic**, split it into a thin `get_x` that holds the extractors and an
  `x(pool, …)` that holds the logic, then `#[sqlx::test]` the latter; drive the real router with
  `scope_test_support::send` / `send_json` for the authorization half. Never add a test-only
  constructor to a permission marker.

## Repos
- Every query lives in `backend/src/repos/`; handlers stay SQL-free. `bin/review-scan.sh` flags SQL
  and `unwrap` in handlers.
- Write paths that must be atomic take `&mut Transaction<'_, Postgres>` (`integration_repo::enqueue_job`)
  so the caller owns the transaction.
- Map a DB error to `ApiError` at the repo boundary; where the cause matters, log it first
  (`code-quality.md` → "Exception Handling"), with the `Display` form (`secure-coding.md` → "Logging").

## Background jobs (`jobs.rs`)
- A new pass is a `spawn_*` that calls `run_interval(reg, name, description, INTERVAL, || …_once(…))`
  and is spawned from `lib.rs`, inside the `if let Some(pool)` block when it needs the database. The
  registry then shows it in the admin jobs view and records each run's outcome.
- The interval and any retention are named constants at the top of `jobs.rs`, with the reason.
- The rest (failure surfacing, replicas, idempotency, testing the `*_once`) is `code-quality.md` →
  "Background passes (`jobs.rs`)".

## The contract

`AGENTS.md` § The API contract → typed client owns the rules, including that a `///` comment on a
`ToSchema` model is contract (#591). Two practical notes:

- Regenerating the client needs no running backend. Dump the spec to a path of your own (concurrent
  worktrees must not share it, #760) and generate from it:

  ```bash
  doc="$(mktemp -d)/openapi.json"
  OIS_OPENAPI_OUT="$doc" cargo test -p ois-backend --lib openapi::dump_openapi_json -- --exact --ignored
  OIS_OPENAPI_URL="$doc" pnpm --filter @ois/api-client codegen
  ```

- Regenerate as the **last** step before committing on any branch that touched a handler signature,
  a `utoipa::path` or a `ToSchema` model, comments included, and diff the result. A regeneration done
  mid-work goes stale. Record the change in `docs-site/reference/api-changelog.md`.

## Realtime
- A mutation that changes what a screen shows publishes its topic with `state.publish(topic::…)`
  **after** the commit (`code-quality.md` → "Queues and transactions").
- A new topic is a `pub const` in `realtime::topic` and an entry in `topic::ALL`, or a client
  subscribing to it is refused as `unknown_topic`; map it to its query keys in `web/src/lib/realtime.ts`.
- A topic carries no payload, ever (`secure-coding.md` → "A check at upgrade or sign-in is a mount
  gate, and a live credential outlives the role").

## The Discord bot and the desktop shell
- `discord/` owns no data. It leases jobs through `POST /api/v1/integration/jobs/lease` and acks with
  the ids it produced. A new bot behavior is a new job kind the backend enqueues, not a database query
  from the bot. The bot registers no slash commands (`AGENTS.md` § Conventions & gotchas).
- `desktop/src-tauri` is a shell around the `web/` bundle. Native capability changes go through
  `secure-coding.md` → "Desktop".
