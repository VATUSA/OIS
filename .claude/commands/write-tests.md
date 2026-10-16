---
description: Audit the branch's tests against its diff, then write the missing ones (Rust unit, #[sqlx::test], router-level, jobs *_once, vitest) and prove each can fail.
---

You are an automated test writer for OIS. Your job is to analyze the branch diff, identify what tests exist and what's missing, then write tests that give REAL confidence the application works correctly.

**First, read `.claude/rules/test-writing.md` in full** (not loaded at startup). `AGENTS.md` § Testing & verification is the harness; this command is the procedure.

## Philosophy

Tests exist to catch bugs BEFORE they reach production. A test that can't catch a bug is worthless. Every test you write must answer YES to all five questions:
1. Would this fail if I introduced a bug in the implementation?
2. Does this test behavior (input → output/side-effect), not implementation details?
3. Is this testing MY code, not Axum, sqlx, serde, React or TanStack internals?
4. Could I delete the implementation and this test would fail?
5. Is there NO other test already covering this exact scenario?

NEVER write these kinds of tests:
- Status-only (`assert_eq!(status, 200)`) or "it rendered" as the only test of something with behavior
- Tautology (the expected value computed by the code under test, or by copying its formula)
- Over-mocked (everything faked, nothing real tested)
- A fixture derived from the constant under test (backdating by `CONST + 1` passes for every value of `CONST`; use absolute values that straddle it)
- A test that reaches a real external API (VATSIM, VATUSA, Open-Meteo, AWC, Discord)

ALWAYS write tests that verify:
- Actual database state after a write
- Real side effects (rows written, a realtime topic published, an outbound job enqueued, a cache reloaded)
- Edge cases (`None`, empty collections, zero, both sides of every boundary)
- Error paths (a missing row, invalid input, an upstream returning nothing or garbage)
- Authorization (a caller without the permission, and for ARTCC-owned data a caller holding it at a different ARTCC; `SERVER_ADMIN` passes every check and proves nothing)
- The wiring (the handler reached through the real router, the job through its `*_once` pass), not just the helper it calls

## Phase 1: Diff Inventory

1. Run `git diff origin/next...HEAD --name-only` to get all changed/added files.
2. Separate files into categories, and the test each one gets:

   | Category | Where it lives | Test it with |
   | --- | --- | --- |
   | Pure logic: trajectory, permission tree, scope math, parsers | `backend/src/feed/`, `crates/ois-core/` | a `#[cfg(test)] mod tests` unit test beside the code |
   | FCA metering | `backend/src/feed/fca.rs` (pure steps), `backend/src/handlers/flow.rs` (orchestration) | unit tests on the steps, plus a router-level test |
   | Repos (SQL) | `backend/src/repos/` | `#[sqlx::test]` against a real throwaway database |
   | Handlers and routes | `backend/src/handlers/`, `router.rs` | a router-level `#[sqlx::test]` through the real router |
   | Migrations | `backend/migrations/` | a `#[sqlx::test]` that exercises the new schema (migrations apply automatically) |
   | The feed | `backend/src/feed/` | a unit test on the pure function over an in-memory snapshot and caches |
   | Jobs | `backend/src/jobs.rs` | `#[sqlx::test]` on the job's `*_once` pass (extract one if the logic is inline) |
   | Bot | `discord/src/` | `#[cfg(test)]` unit tests on the pure payload and parsing logic; nothing talks to Discord |
   | Web | `web/src/`, `packages/ui/` | vitest `*.test.ts(x)`; DOM tests opt in with a `// @vitest-environment jsdom` first line and are usually named `*.dom.test.ts(x)` |
   | "X never happens" (nothing stores a token, no route calls Y) | anywhere | a source-scan guard (`*.guard.test.ts`), proven by planting the forbidden call in a throwaway file |

3. For each category, identify WHAT changed (read the actual code, not just filenames).

## Phase 2: Test Audit

1. For each changed/added source file, find its tests and read them in full:
   - Rust tests sit in the same file's `#[cfg(test)]` module or in a `backend/src/handlers/*_tests.rs` file registered in `backend/src/handlers/mod.rs`.
   - Web tests sit beside the component as `*.test.ts(x)`.
   - A web page or component with genuine browser behavior gets BOTH a vitest DOM test AND a run of the real page against the stack (Phase 4); apply `.claude/rules/test-quality.md` → "jsdom vs the running stack — the decision rule" first. OIS has no browser-automation suite: never invent one.

2. For tests that DO exist, evaluate quality:
   - Are they testing real behavior or just a status or a render?
   - Do they verify database state changes?
   - Do they cover edge cases and a sad path?
   - Do they test authorization (missing permission, wrong ARTCC) through the router?
   - **Do they test the wiring?** A helper tested directly while the handler, job or scheduler pass that calls it goes untested is the gap OIS has shipped most often: the fix lands in the helper, the caller never calls it, every test stays green.
   - For a new or changed endpoint, does `backend/src/handlers/auth_annotation_tests.rs` still pass? It checks every `#[utoipa::path]` handler's advertised auth against its extractors.
   - For a trajectory change, is each caller `AGENTS.md` § The trajectory / ETA model lists tested, not only `trajectory.rs`?

3. Produce a gap report showing:
   - Files with NO test coverage
   - Files with SHALLOW coverage (status-only, render-only)
   - Web files with behavior but no DOM test, and DOM tests that assert nothing a user sees
   - Missing edge-case, sad-path and authorization tests
   - Missing wiring tests (router, `*_once` pass, the full chain)

If you only need the audit, dispatch `test-reviewer` instead and stop here.

## Phase 3: Write Tests

For each gap identified, write the appropriate test. Mirror the nearest existing test of the same kind; never invent a new harness.

### For Pure Logic (Unit Tests)

Location: the module's own `#[cfg(test)] mod tests`.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // 300 nm at a 450 kt groundspeed: 300 / 450 * 3600 = 2400 s
    #[test]
    fn a_cruise_leg_is_timed_at_its_groundspeed() {
        assert_eq!(leg_seconds(300.0, 450.0), 2400.0);
    }

    #[test]
    fn a_zero_groundspeed_leg_has_no_time() {
        assert_eq!(leg_seconds(300.0, 0.0), 0.0); // or whatever the spec says it must do
    }
}
```

Name the test for the behavior, not the function. Use absolute inputs and expected values worked out by hand, with the arithmetic in a comment.

### For Repos and Migrations (`#[sqlx::test]`)

Each test gets its own database with every migration applied (`backend/src/repos/access.rs` has examples). Seed with plain `insert … returning id`.

```rust
#[sqlx::test]
async fn pruning_keeps_the_rows_outside_its_window(pool: PgPool) {
    // Seed one row that must go, and one surviving neighbor per predicate of the WHERE.
    // Act: call the repo function.
    // Assert: the target row is gone AND every neighbor is still there.
}
```

For a destructive `WHERE`, seed one row that must **survive** for every predicate, or a test against a one-row table passes for a query that deletes everything.

### For Jobs (Run the Pass for REAL)

Test the code the scheduler actually runs, not only the repo function it calls. Many jobs have a `*_once` pass; test it with `#[sqlx::test]`, times anchored on `Utc::now()` (`mod ace_reminder_tests` in `backend/src/jobs.rs` is the model). Others keep their logic, cutoff arithmetic included, inline in the `spawn_*` closure: move it into a `*_once` pass first, taking `now` as a parameter so a boundary test can fix the time (`diagnostics_report_prune_once` is the shape), then test that.

### For Handlers and Routes (Router-Level)

Drive the real router. `backend/src/scope_test_support.rs` has `test_state`, `seed_user`, `grant`, `deny_scoped`, `session_cookie`, `send` (any method and JSON body, returns the status) and `send_json` (any method, no request body, returns the status and the decoded body). For a machine caller's bearer request, `backend/src/handlers/machine_actor_tests.rs` has the `api_key`/`call_with` helpers.

```rust
#[sqlx::test]
async fn a_zdc_scoped_editor_cannot_change_a_zny_config(pool: PgPool) {
    // seed_user + grant the permission scoped to ZDC; session_cookie
    // send(PUT the ZNY-owned record) → assert the status
    // AND assert the row is unchanged in the database
}
```

Assert the status **and** what changed: the body for a read, the database for a write. For a permissioned route, three tests at least: allowed, missing permission, and (for ARTCC-scoped data) the permission at the wrong ARTCC. For those handlers a missing permission is 401 and a wrong facility 403; assert which one fired.

### For the Feed

The feed has no DB handle, so test the pure function over an in-memory snapshot and caches. For a cache-plus-refresh-job pattern, test the write handler's force-reload too, not just the loader.

### For Web (vitest)

Seed the TanStack Query cache with `queryClient.setQueryData(...)` rather than stubbing `fetch`: `web/src/test/no-network.ts` blocks the network, and the generated client captures `fetch` when it loads, so a stub in the test is too late. To assert what is sent, mock the client module (`vi.mock("@/lib/api", …)`) and assert the call, as `web/src/pages/admin/service-accounts.dom.test.tsx` does for a write. Use the generated client's types for fixtures so a contract change breaks the test at compile time. Assert what the user sees and what is sent, not component internals. An `await` on an optimistic write can race its rollback: assert on the settled state.

### For E2E Chain Tests (CRITICAL — One Per Cross-System Feature)

A change whose output another part of OIS consumes gets one test that runs the whole chain unfaked through the router: an endpoint that enqueues a Discord job (assert the `integration.outbound_jobs` row the bot would lease), a write that publishes a realtime topic (`backend/src/handlers/topic_publish_tests.rs` is the pattern), a write that a feed cache must pick up (assert the reloaded cache). Assert every step's outcome, not only the last.

## Phase 4: Run and Verify

### Targeted runs
1. Run each new test: `cargo nextest run -p <crate> -E 'test(<name>)'` (or `bash bin/run-tests.sh rust -p <crate> -E 'test(<name>)'`, which never stamps) and `pnpm --filter <package> exec vitest run <path>`. They MUST pass. Read nextest's summary line (not `0 tests run` from a filter typo) or vitest's `Test Files` and `Tests` lines (not "No test files found").
2. If a test fails, read the implementation to understand whether the test or the implementation is wrong. Fix the TEST if the implementation is correct, fix the IMPLEMENTATION if the test reveals a bug, and say so.
3. **Mutate**: for each new test, make the smallest plausible bug in the code it guards (flip a comparison, drop a `RequirePermission`, remove a predicate), run the test, watch it go red, then restore. Use `bin/mutate.sh FILE OLD NEW -- COMMAND` with a narrow COMMAND (the one test): it prints `RED` or `GREEN` and restores the file byte for byte. Mutating by hand, commit a checkpoint first (`git checkout -- <file>` restores HEAD and would wipe uncommitted work) and confirm the mutation applied (`git diff --quiet` must fail). Never mutate while a suite is building in the same tree. A test that stays green under a plausible mutation is decoration; strengthen it or delete it.

### The real page (whenever the change has a page a user looks at)
4. Run the page against the stack: `just backend` on a throwaway database (`.claude/rules/database-postgres.md` → "Running against a real database"), `just web`. Exercise what you changed; record the URL and what you saw. A DOM test proves the component; this proves the page, the route and the generated client agree.

### The full gate
5. Run `just test` (`bin/run-tests.sh`, in the background, output to a file) once the tests are committed: it runs every gate CI runs that can run here and stamps the tests marker for a green run on a clean commit. Run it once per SHA; never again on a SHA that already has a tests marker or green CI.
6. `cargo fmt --all` and `pnpm lint` before committing.

A task is NOT complete until the full gate is green and, for a UI change, the page has been run.

## Rules

### General
- NEVER define setup helpers in a test file beyond what `scope_test_support.rs` and `machine_actor_tests.rs` already offer — repetition is clearer
- NEVER mock the thing you're testing
- NEVER hit real external APIs (VATSIM, VATUSA, Open-Meteo, AWC, Discord)
- Never change a test's expected value to make it pass; find out whether the test or the code is wrong

### Rust tests
- DB behavior in `#[sqlx::test]`, never against the shared dev database
- Authorization tests go through the router: calling a handler function directly skips its extractors
- For EVERY job: at least ONE test on the pass the scheduler runs
- For EVERY trajectory change: a test at each caller it affects
- Name an unused pool `_pool`, so clippy's `-D warnings` stays clean

### Web tests
- vitest, beside the code; DOM tests opt in per file with `// @vitest-environment jsdom`
- Seed the query cache; never stub `fetch`
- An absence property ("nothing persists the token") is a `*.guard.test.ts` source scan, proven by injecting the forbidden call into a new throwaway file

## Priority Order

Write tests in this order (highest impact first):
1. **The wiring and authorization of new routes** — router-level, allowed / missing permission / wrong ARTCC
2. **E2E chain tests** — handler → outbound job, handler → realtime topic, write → feed cache
3. **Repo writes and migrations** — `#[sqlx::test]`, a surviving neighbor per predicate
4. **The trajectory model and every caller** — FCA metering, airport-flow demand, runway ETE, sector occupancy
5. **Job `*_once` passes**
6. **Pure logic** — fast feedback
7. **Web behavior** — DOM tests plus the real page

## Output

After writing all tests, provide a summary:
- Number of Rust test functions and web test files created, by layer
- Coverage areas addressed, against the Phase 2 gap report
- Each mutation and whether it went red
- Any bugs discovered during test writing (and whether you fixed them)
- The targeted runs' summary lines, the `just test` result (or the tests marker or green CI it relied on), and the real-page line (URL and what was seen) or "no UI change"
