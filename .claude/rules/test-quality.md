# Test quality

Loads for every file. What counts as test evidence and how to prove a test can fail. The test
commands, the `#[sqlx::test]` setup, and the "never hit real external APIs" rule are in `AGENTS.md`
§ Testing & verification and § Commands. Stack-specific test mechanics are in `rust-backend.md`
and `web-frontend.md`.

Sources: ported from the house test-quality rule; OIS lessons from #419, #428, #433, #457, #472,
#488, #509, #510, #531, #550, #577, #585, #656, #699, and #706.

## Read the summary line

The exit code lies in both directions. Read `test result:` for cargo and the `Test Files` /
`Tests` lines for vitest; nothing else is evidence. Background-task summaries have reported "exit
code 0" for logs that ended in a failure.

- **Name the failing test before you reason about it.** Keep the full log (redirect to a file in
  your scratchpad, don't pipe the run through `tail`) and pull the block with
  `sed -n '/^failures:$/,/^$/p'`.
- **turbo can replay another worktree's cached log.** `pnpm test`, `pnpm typecheck`, and
  `pnpm lint` may print `cache hit, replaying logs`, and the replayed log can show another
  worktree's absolute path. When the result matters, run with `--force` and confirm
  `cache bypass, force executing`, or run `pnpm vitest run` in the package directly.

## A gate you didn't run is not a gate that passed

`just ci` skips clippy, vitest, doc tests, `pnpm audit`, `cargo deny`, and `client-drift`; see
`AGENTS.md` § Commands. On #457 `just ci` was green with 589 passing while CI's clippy job failed
on the branch's own defect.

- In every report, PR, and issue comment, say which gates ran and which did not, and why.
- Before trusting a quiet harness, watch it pass one test you expect to pass. A harness that
  cannot start looks the same as a clean run.
- Before reporting a CI verdict, read the real check runs:
  `gh api repos/VATUSA/OIS/commits/<sha>/check-runs --jq '.check_runs[] | "\(.conclusion) \(.name)"'`.

## Flake or regression

Agents gate in parallel worktrees on one CPU and one Postgres, and that produces two environmental
failure shapes that look like real ones.

- **Starvation looks like a hang.** Before deciding a run is stuck, check `pgrep -fl 'cargo test'`,
  `pgrep -c rustc`, and `uptime`.
- **The `#[sqlx::test]` harness flakes under contention.** The panic sits inside `sqlx-core`
  (`database "_sqlx_test_…" does not exist`, `PoolTimedOut`, `already exists`), never in an
  assertion, and it lands on a different unrelated test each run. Use `just test-rust`, which runs
  with `--test-threads=1` for this reason (`justfile:66-67`).
- **Classify by isolation.** Re-run each named failure on its own. A flake does not survive
  isolation; a real failure repeats and sits in code you touched. Say in the PR which failures
  were flakes and that you re-ran them.
- **Classify red as pre-existing or introduced before you start work**, against `origin/next`.

## Prove the test can fail: mutation

A green test proves the assertion ran, not that it could ever fail. Break the code and watch the
test go red: delete the guard, invert the condition, drop the predicate. Then restore. This is
mandatory for a permission or scope check, a destructive statement, a time window, a dedup key, a
tuning constant, and any flaky test.

Doing it safely:

- **Commit a WIP checkpoint before the first mutation**, and again whenever a mutation makes you
  write new code. `git checkout -- <file>` restores HEAD, not "before the mutation", and has
  deleted uncommitted tests mid-cycle (#472). Restore from a `cp` backup in your scratchpad and
  `grep` for your new test's name after every restore.
- **Assert each mutation applied** (`git diff --quiet` means it did not), and re-check mutation
  patterns after `cargo fmt` reflows lines.
- **Don't mutate while a suite builds in the same tree.** cargo and vitest read sources at build
  time, so a background run picks up your mutations (#699). Mutate first, or in a second worktree.
- **One verification run per worktree.** To abandon a backgrounded script, stop the task itself,
  not its test binary; killing the child lets the shell run its next mutation (#585).
- **Before trusting the result**, check the tree matches the commit under review:
  `git status --porcelain` is empty, or `git diff --quiet <sha> -- .`.

## What mutation proves, and what it doesn't

- **A killed mutation proves the test pins the behavior, not that the behavior is right.** Check
  the spec separately; see `code-quality.md` § A "mirror of X" fix (#488).
- **Pin the constant with absolute fixtures.** A fixture built from the constant under test moves
  with it: backdating a row by `RETAIN_HOURS + 1` stayed green when the retention went from 12 h
  to 8,760 h (#509). Use fixtures that straddle the documented value (11 h and 13 h) plus an
  assert that the constant still sits between them, then mutate the constant itself.
- **Test the far side of a boundary.** For `(a - b).abs() <= WINDOW`, a value just inside the
  window passes with `.abs()` deleted, because a negative satisfies `<= n` on its own (#510).
  Assert that both far sides reject and one inside accepts, and keep fixtures clear of truncation
  edges (`num_minutes()` truncates).
- **Mutate every predicate of a destructive `WHERE`.** List the predicates and delete each in
  turn; each deletion must fail a test. Start the fixture from the most common real row, then seed
  **one surviving neighbor per predicate** (same ARTCC and another sector; same sector and another
  ARTCC). A `WHERE` tested against a one-row table proves nothing (#550, #656, #706).
- **A second gate masks the first.** ANDing a new check onto a guard makes the old guard's tests
  pass under the old mutations, because the new check now refuses the same inputs (#577). After
  adding a gate, re-run every earlier mutation, and give each half of the AND a fixture where the
  other half passes.

## Test the wiring, not just the unit

In OIS the regression is usually a call site drifting, not the unit misbehaving. Five returns in
one session (#428, #433 twice, #419) had a well-tested unit and an untested caller; reverting the
real bug left the suite green. **Before shipping, revert the actual bug and run the suite.** If it
stays green, the test does not defend the fix. Four shapes have precedent:

- **A source scan over call sites** (`web/src/router.window-chrome.test.ts`,
  `web/src/components/sign-in-button.guard.test.ts`).
- **A handler-level `#[sqlx::test]`.** If the extractors get in the way, split the handler into a
  thin `get_x` and an `x(pool, …)` that holds the logic; never add a test-only constructor to a
  permission type.
- **A real request through the real router** with `scope_test_support::send`
  (`backend/src/scope_test_support.rs:162`). It returns a `StatusCode` only, so it covers authz
  gates, not response bodies.
- **A test on the `jobs.rs` pass itself** when a background job is the writer. The private
  `*_once` functions are the recurring blind spot; `mod ace_reminder_tests`
  (`backend/src/jobs.rs:1711`) is the precedent. Anchor times on `Utc::now()`, not a calendar date.

## Absence needs a guard, not a grep

An acceptance criterion that a side effect is **absent** ("nothing persists the token client-side",
"never calls the network") can't be verified by grepping once; that proves today, not tomorrow.
On #531 the grep was clean, and a reviewer then added a `localStorage.setItem` of the token with
all tests green. Pin it with a source-scan test named `*.guard.test.ts` (see `web/src/lib/` and
`web/src/components/`):

- scan every file the value flows through, not just the component
- strip comments first, because prose that mentions the forbidden call is not a call
- include a negative case, or a matcher that stops matching passes forever
- prove it by injecting the forbidden call into a **new throwaway file**, never a real one
- say in a comment why it is a source scan, so nobody swaps it for something weaker

## Assertions that look stronger than they are

- **Assert every clause of the test's name.** "X does not happen when Y" promises two things, and
  the negative half is the one usually missing.
- **Every "must not appear" needs a positive control** in the same test, proving the mechanism ran.
- **Assert the exact property**: the count, the order, the boundary value, not "non-empty".
- **Don't recompute the formula under test.** Take one side from the code and the other from the
  requirement, as a literal.
- **When the guard path and the failure path return the same value**, assert a side effect that
  differs (a log, a row, a call that did not happen).
- **Test a parser against bytes the real producer emitted**, captured to a file, never a sample
  you typed. Check for escapes with `cat -v` before writing the pattern.

## Five questions before keeping a test

1. Would it fail if I introduced a bug?
2. Does it test behavior or implementation?
3. Is it testing my code, not the framework or a library?
4. Could I delete the implementation and still see it pass?
5. Does another test already cover this exact scenario?

Delete or rewrite on a wrong answer. Mock-returning-mock, tautologies, render-only checks,
assertion-free tests, and constructor/getter tests are the usual culprits.
