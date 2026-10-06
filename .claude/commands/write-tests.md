---
description: Audit the branch's tests against its diff, then write the missing ones (Rust unit, router-level, #[sqlx::test], vitest) and prove each can fail.
argument-hint: "[path or base ref] (default: origin/next...HEAD)"
---

You are writing tests for the current branch. Analyze the diff, find what is untested or tested
without proof, and write tests that give **real** confidence the change works. The standard is
`.claude/rules/test-quality.md`; `AGENTS.md` § Testing & verification is the harness. This command is
the procedure, not a second copy of either.

## The bar

A test earns its place only if every answer is yes:

1. Would it fail if I introduced a plausible bug in the implementation?
2. Does it test behavior (input → output or side effect), not implementation details?
3. Is it testing OIS code, not Axum, sqlx, serde or React?
4. Would it fail if the implementation were deleted or stubbed to return a default?
5. Is no other test already covering this exact scenario?

Never write: a test whose only assertion is a 200 or "it rendered"; a tautology; a test that mocks
the thing under test; a fixture derived from the constant under test (backdating by `CONST + 1` passes
for every value of `CONST`; use absolute values that straddle it); a test that hits a real external API
(VATSIM, VATUSA, Open-Meteo, AWC).

Always cover: the database state after a write; both sides of every boundary; the error paths
(missing row, invalid input, an upstream returning nothing); **authorization** (a caller without the
permission, and for ARTCC-owned data a caller holding it at a different ARTCC); and the wiring (the
handler reached through the router, not only the helper it calls).

## Phase 1 — Inventory

`git diff origin/next...HEAD --name-only` (or the range in `$ARGUMENTS`), then sort the changed files:

| Kind | Where it lives | Test it with |
| --- | --- | --- |
| Pure logic: trajectory, metering, permission tree, parsers | `backend/src/feed/`, `crates/ois-core/` | a `#[cfg(test)] mod tests` unit test beside the code |
| SQL | `backend/src/repos/` | `#[sqlx::test]` against a real throwaway database |
| Handlers and routes | `backend/src/handlers/`, `router.rs` | a router-level test through `build_router` |
| Migrations | `backend/migrations/` | a `#[sqlx::test]` that exercises the new schema (migrations apply automatically) |
| Jobs and the feed | `backend/src/jobs.rs`, `backend/src/feed/` | the pure step as a unit test, and the scheduler pass that calls it |
| Bot | `discord/src/jobs/`, `discord/src/interactions/` | `#[cfg(test)]` unit tests on the pure payload and parsing logic; nothing talks to Discord |
| Web | `web/src/`, `packages/ui/` | vitest `*.test.ts(x)`, DOM tests opting in with `// @vitest-environment jsdom` |

Read what each file actually changed, not just its name.

## Phase 2 — Audit what exists

For each changed source file, find its tests and read them in full:

- Is there a test at all? Rust tests sit in the same file's `#[cfg(test)]` module or in a
  `backend/src/handlers/*_tests.rs` file; web tests sit beside the component as `*.test.ts(x)`.
- Does it assert data, not just status? Does it cover a sad path and an authorization failure?
- **Does it test the wiring?** A helper tested directly while the handler, job or scheduler pass that
  calls it goes untested is the gap OIS has shipped most often: the fix lands in the helper, the caller
  never calls it, every test stays green.
- For a new or changed endpoint, does `backend/src/handlers/auth_annotation_tests.rs` still pass? It
  checks every `#[utoipa::path]` handler's advertised auth against its extractors.

Write a gap report: files with no tests, files with shallow tests, and missing sad-path,
authorization and wiring tests. If you only need the audit, dispatch `test-reviewer` instead and stop
here.

## Phase 3 — Write the tests

Mirror the nearest existing test of the same kind; never invent a new harness.

**Pure logic** — a unit test in the module's `#[cfg(test)] mod tests`. Name it for the behavior
(`descent_uses_the_descent_schedule_not_cruise`), not the function. Use absolute inputs and expected
values worked out by hand, with the arithmetic in a comment.

**Repos and migrations** — `#[sqlx::test]` with a `pool: sqlx::PgPool` argument; each test gets its own
database with every migration applied (`backend/src/repos/access.rs` has examples). Seed with plain
`insert … returning id`. For a destructive `WHERE`, seed one row that must **survive** for every
predicate, or a test against a one-row table passes for a query that deletes everything.

**Handlers** — drive the real router: `crate::router::build_router(state).oneshot(request)`.
`backend/src/scope_test_support.rs` has `send` and `send_json` and a minimal `AppState`. Assert the
status **and** the body or the database afterwards. For a permissioned route, three tests at least:
allowed, missing permission, and (for ARTCC-scoped data) the permission at the wrong ARTCC. A missing
permission and a wrong scope answer differently; assert which one fired.

**Feed and jobs** — the feed has no DB handle, so test the pure function over an in-memory snapshot
and caches. For a cache-plus-refresh-job pattern, test the write handler's force-reload too, not just
the loader.

**Web** — vitest. Seed the TanStack Query cache with `queryClient.setQueryData(...)` rather than
stubbing `fetch`. Use the generated client's types for fixtures so a contract change breaks the test
at compile time. Assert what the user sees and what is sent, not component internals.

## Phase 4 — Run, then prove each test can fail

1. Run the new tests: `cargo test -p <crate> <test_name>` or `pnpm --filter web test -- <file>`.
   Read the `test result:` line; it must show them passing, not `0 passed` from a filter typo.
2. **Mutate**: for each new test, make the smallest plausible bug in the code it guards (flip a
   comparison, drop a `RequirePermission`, remove a predicate), run the test, watch it go red, then
   restore. Commit a checkpoint first: `git checkout -- <file>` restores HEAD and would wipe uncommitted
   work. A test that stays green under a plausible mutation is decoration; strengthen it or delete it.
3. If a test reveals a real bug, fix the implementation, not the test, and say so.
4. Run `just ci-full` before calling it done.

## Priority

Highest value first: the wiring and authorization of new routes; repo writes and migrations; the
trajectory model and its three callers (FCA metering, airport-flow demand, runway ETE); pure logic;
web behavior.

## Report

Files and tests added, the gaps they close, each mutation and whether it went red, bugs found (and
whether fixed), and the `just ci-full` summary.
