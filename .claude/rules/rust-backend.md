---
paths:
  - "backend/**"
  - "crates/**"
  - "discord/**"
  - "desktop/src-tauri/**"
---

# Rust backend

Loads when you read or edit Rust workspace files. The architecture (handler → repo → model, the
permission markers and their three-in-sync rule, the API contract and client regeneration, the
trajectory model's three callers, the feed's no-DB rule, realtime, auditing) is in `AGENTS.md`
§ Architecture, and the error envelope and auth model are in § Conventions & gotchas. Read those;
this file adds what has bitten Rust changes and does not repeat them.

Sources: OIS lessons from #433, #436, #457, #508, #591, #537, and the `just ci` / CI comparison
in `AGENTS.md` § Commands.

## Clippy is the gate `just ci` skips

Run `cargo clippy --workspace --all-targets -- -D warnings` on every Rust change (CI runs it at
`.github/workflows/ci.yml:63`). Two traps that have passed `just ci` and failed CI:

- **`items_after_test_module`.** Appending new code below an existing `#[cfg(test)] mod tests`
  fails the lib-test build (#457). Put implementation above the test module.
- **`too_many_arguments`.** Adding one parameter to a 7-argument function fails the lint (#508).
  Group related parameters into a struct rather than reaching for `#[allow]`; it also stops call
  sites mixing up several same-typed references.

## Running the Rust tests

- Use `just test-rust`, not bare `cargo test --workspace`. The recipe passes `--test-threads=1`
  (`justfile:66-67`) because parallel `#[sqlx::test]` databases collide. CI uses nextest with its
  own profile; locally the single thread is what keeps the harness honest.
- Classifying harness flakes versus real failures is in `test-quality.md` § Flake or regression.

## Handlers and repos

- **Every mutating handler takes `RequirePermission<P>`**, plus the data-dependent check on top;
  see `secure-coding.md`.
- **SQL lives in `repos/`** and binds every value; see `database-postgres.md`.
- **Heavy CPU off the async workers.** Route resolution, metering, and similar work run under
  `tokio::task::spawn_blocking` (`backend/src/handlers/flow.rs:621`). Holding a runtime worker
  for long CPU work stalls every request on it.
- **No silent skips.** `if let Some(x) = lookup { … }` with no `else` turns misconfiguration into
  a quiet no-op; on #436 that was a 200 with nothing enqueued and nothing logged. Log the `else`
  with `tracing::warn!` and decide whether the caller should see an error.
- **To test a handler's logic**, split it into a thin `get_x` that holds the extractors and an
  `x(pool, …)` that holds the logic, then `#[sqlx::test]` the latter. Never add a test-only
  constructor to a permission marker. For authz gates, `scope_test_support::send`
  (`backend/src/scope_test_support.rs:162`) drives the real router and returns only a status code.

## Background jobs (`jobs.rs`)

- **A pass must surface failure.** Return `Err(detail)` so the job registry records it
  (`backend/src/job_registry.rs:148`) and `/metrics` reports it. Logging and returning `Ok` hides
  it.
- **Split the pass into a testable `*_once(pool)`** and test that directly. The private `*_once`
  functions have no route and nothing reaches them by accident, so they are the usual untested
  writer (#433). `mod ace_reminder_tests` (`backend/src/jobs.rs:1711`) is the precedent.
- **Anchor job-test fixtures on `Utc::now()`**, not a calendar date. The passes compare against
  `now()`, and a hard-coded date silently drifts out of every window it relied on.
- **A job acting for a user has a principal.** A pass that publishes or mutates on a user's behalf
  carries the row's actor (for example `updated_by`), and the permission check applies there too
  (#537). Read the job before claiming it "has no principal".
- **A tuning constant is a behavior change.** Retention and timeout constants get "tuned" without
  anyone noticing what they turn off (`backend/src/jobs.rs:1318`). Pin them as
  `test-quality.md` § Pin the constant describes.

## The contract

`AGENTS.md` § The API contract → typed client owns the rules, including that a `///` comment on a
`ToSchema` model is contract (#591). Two practical notes:

- Regenerating the client needs no running backend. CI dumps the spec offline
  (`.github/workflows/ci.yml:139-145`); run the same three commands locally.
- Regenerate as the **last** step before committing on any branch that touched a `ToSchema` model,
  comments included, and diff the result. A regeneration done mid-work goes stale.

## Changing a shared shape

After changing an enum variant, a public function signature, or a shared type, grep the whole
workspace for the old shape. Sibling branches merged alongside yours still use it, and only an
`--all-targets` build of `next` will tell you; see `git-and-worktrees.md` § Merging a batch.

## The Discord bot and the desktop shell

- `discord/` owns no data. It leases jobs through the API and acks with the ids it produced
  (`AGENTS.md` § Project overview). A new bot behavior is a new job kind enqueued by the backend,
  not a database query from the bot.
- `desktop/src-tauri` is a shell around the `web/` bundle. Native capability changes go through
  `secure-coding.md` § Desktop.
