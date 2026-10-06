---
description: Audit the branch's tests against its diff, then write the missing ones (Rust unit, router-level, #[sqlx::test], vitest) and prove each can fail.
argument-hint: "[path or base ref] (default: origin/next...HEAD)"
---

You are writing tests for the current branch. Analyze the diff, find what is untested or tested
without proof, and write tests that give **real** confidence the change works. The standard is
`.claude/rules/test-quality.md`; `AGENTS.md` § Testing & verification is the harness. This command is
the procedure, not a second copy of either. Read `test-quality.md` first.

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
| Pure logic: trajectory, permission tree, parsers | `backend/src/feed/`, `crates/ois-core/` | a `#[cfg(test)] mod tests` unit test beside the code |
| FCA metering | `backend/src/feed/fca.rs` (pure steps), `backend/src/handlers/flow.rs` (orchestration) | unit tests on the steps, plus a router-level test |
| SQL | `backend/src/repos/` | `#[sqlx::test]` against a real throwaway database |
| Handlers and routes | `backend/src/handlers/`, `router.rs` | a router-level test through `build_router` |
| Migrations | `backend/migrations/` | a `#[sqlx::test]` that exercises the new schema (migrations apply automatically) |
| The feed | `backend/src/feed/` | a unit test on the pure function over an in-memory snapshot and caches |
| Jobs | `backend/src/jobs.rs` | `#[sqlx::test]` on the job's `*_once` pass (extract one if the logic is inline) |
| Bot | `discord/src/jobs/`, `discord/src/interactions/` | `#[cfg(test)]` unit tests on the pure payload and parsing logic; nothing talks to Discord |
| Web | `web/src/`, `packages/ui/` | vitest `*.test.ts(x)`; DOM tests opt in with a `// @vitest-environment jsdom` first line and are usually named `*.dom.test.ts(x)` |
| "X never happens" (nothing stores a token, no route calls Y) | anywhere | a source-scan guard (`*.guard.test.ts`), proven by planting the forbidden call in a throwaway file |

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
`backend/src/scope_test_support.rs` has a minimal `test_state`, `seed_user`, `grant`, `deny_scoped`,
`session_cookie`, and `send` (any method and JSON body, returns the status) and `send_json` (any
method, no request body, returns the status and the decoded body). Assert the status **and** what
changed: the body for a read, the database for a write. For a permissioned route, three tests at
least: allowed, missing permission, and (for ARTCC-scoped data) the permission at the wrong ARTCC.
For those handlers a missing permission is 401 and a wrong facility 403; assert which one fired.

**Feed** — the feed has no DB handle, so test the pure function over an in-memory snapshot and
caches. For a cache-plus-refresh-job pattern, test the write handler's force-reload too, not just the
loader.

**Jobs** — test the code the scheduler actually runs, not only the repo function it calls; that gap
is where OIS fixes have gone untested before. Many jobs have a `*_once` pass; test it with
`#[sqlx::test]`, times anchored on `Utc::now()` (`mod ace_reminder_tests` in `backend/src/jobs.rs` is
the model). Others keep their logic, cutoff arithmetic included, inline in the `spawn_*` closure:
move it into a `*_once` pass first, taking `now` as a parameter so a boundary test can fix the time
(`diagnostics_report_prune_once` is the shape), then test that.

**Web** — vitest. Seed the TanStack Query cache with `queryClient.setQueryData(...)` rather than
stubbing `fetch`: `web/src/test/no-network.ts` blocks the network, and the generated client captures
`fetch` when it loads, so a stub in the test is too late. To assert what is sent, mock the client
module (`vi.mock("@/lib/api", …)`) and assert the call, as
`web/src/pages/admin/service-accounts.dom.test.tsx` does for a write. Use the
generated client's types for fixtures so a contract change breaks the test at compile time. Assert
what the user sees and what is sent, not component internals.

## Phase 4 — Run, then prove each test can fail

1. Run the new tests: `cargo test -p <crate> <test_name>` or `pnpm --filter web test -- <file>`.
   Read cargo's `test result:` line (not `0 passed` from a filter typo) or vitest's `Test Files` and
   `Tests` lines (not "No test files found").
2. **Mutate**: for each new test, make the smallest plausible bug in the code it guards (flip a
   comparison, drop a `RequirePermission`, remove a predicate), run the test, watch it go red, then
   restore. Commit a checkpoint first: `git checkout -- <file>` restores HEAD and would wipe uncommitted
   work. Confirm the mutation applied (`git diff --quiet` must fail), and never mutate while a suite is
   building in the same tree. A test that stays green under a plausible mutation is decoration;
   strengthen it or delete it.
3. If a test reveals a real bug, fix the implementation, not the test, and say so.
4. Run `just ci-full` before calling it done.

## Priority

Highest value first: the wiring and authorization of new routes; repo writes and migrations; the
trajectory model and every caller (`git grep -n 'trajectory::'`: FCA metering, airport-flow demand,
runway ETE, sector occupancy); job `*_once` passes; pure logic;
web behavior.

## Report

Files and tests added, the gaps they close, each mutation and whether it went red, bugs found (and
whether fixed), and the `just ci-full` summary.
