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
trajectory model and every caller it reaches, the feed's no-DB rule, realtime, auditing) is in
`AGENTS.md` § Architecture, with the callers listed under § The trajectory / ETA model, and the
error envelope and auth model are in § Conventions & gotchas. Read those; this file adds what has
bitten Rust changes and does not repeat them.

Sources: OIS lessons from #433, #436, #457, #508, #591, #537, #725, and the `just ci` / CI comparison
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

- Locally, use `just test-rust`, not bare `cargo test --workspace`. The recipe passes
  `--test-threads=1` (`justfile:66-67`): many threads in one `cargo test` process have collided on
  `#[sqlx::test]` databases (`_sqlx_test_… already exists`, 2026-09-30). `cargo nextest run`, which
  CI uses, runs each test in its own process and is safe in parallel (`.config/nextest.toml`,
  `AGENTS.md` § Testing & verification).
- Classifying harness flakes versus real failures is in `test-quality.md` § Flake or regression.

## Handlers and repos

- **Every mutating handler takes `RequirePermission<P>`**, plus the data-dependent check on top;
  see `secure-coding.md`.
- **SQL lives in `repos/`** and binds every value; see `database-postgres.md`.
- **Heavy CPU off the async workers.** Route resolution, metering, and similar work run under
  `tokio::task::spawn_blocking` (`fca_counts` in `backend/src/handlers/flow.rs`). Holding a runtime worker
  for long CPU work stalls every request on it.
- **A read over feed data computes once per snapshot, not per request.** `spawn_blocking` moves
  the cost off the async workers but doesn't remove it. A handler that projects, routes, or bins
  the feed's flights repeats that work for every viewer, every open table, and every
  topic-triggered refetch. Compute it once per feed snapshot into an `AppState` cache keyed by the
  snapshot and the versions of the config it reads, refresh it the way feed-visible config is
  refreshed (`AGENTS.md` § Conventions & gotchas, "Config that must reach the feed": a refresh
  job, plus a force-reload on write), and have the handler only read and slice the cached result.
  On #725 the sector-demand read recomputed six hours of projection per request: 0.25–0.8 s of CPU
  per ARTCC in release, about 97% of it in one per-minute `distance_after` loop.
- **Measure cost in a release build, then name the hot spot.** A debug build overstated #725's
  cost about tenfold (5.9–9.3 s, against 0.25–0.8 s in release). Bench with `--release` against a
  captured live feed, and give the `file:line` that dominates before putting a cost to the owner.
- **No silent skips.** `if let Some(x) = lookup { … }` with no `else` turns misconfiguration into
  a quiet no-op; on #436 that was a 200 with nothing enqueued and nothing logged. Log the `else`
  with `tracing::warn!` and decide whether the caller should see an error.
- **To test a handler's logic**, split it into a thin `get_x` that holds the extractors and an
  `x(pool, …)` that holds the logic, then `#[sqlx::test]` the latter. Never add a test-only
  constructor to a permission marker. `scope_test_support::send` (`backend/src/scope_test_support.rs:162`)
  drives the real router and returns the status; `send_json` (`:193`) also returns the body.

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
  `test-quality.md` § What mutation proves describes (pin the constant).

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
