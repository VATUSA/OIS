# Test Quality Rules (always loaded)

This file covers running tests and reading their results, tiering, the running stack, and the
mutation discipline. Writing tests lives in `.claude/rules/test-writing.md` (not loaded at startup;
read it in full before writing, changing or reviewing a test): the pyramid, coverage, FIRST, naming,
STUPID/VALID, the Five Questions, mocking and fixture rules, router-level tests for authorization
gates, `#[sqlx::test]` and `jobs.rs` pass tests, vitest + jsdom, source-scan guards, testing a parser
against captured bytes, boundary and destructive-`WHERE` fixtures, and time injection.

Test mechanics (the `#[sqlx::test]` setup, "never hit real external APIs") are in `AGENTS.md`
§ Testing & verification; the commands are in `AGENTS.md` § Commands.

---

## Testing Framework
- **Rust:** `cargo nextest run` (CI uses `--profile ci`, `.config/nextest.toml`). Each test runs in
  its own process, so the suite is safe in parallel. DB-touching tests are `#[sqlx::test]`: real
  Postgres, one throwaway database per test, migrations applied automatically.
- **Web:** vitest in `web` and `packages/ui` (`@ois/ui`). The default environment is node; a DOM
  test opts in with `// @vitest-environment jsdom` on line 1. `web/src/test/no-network.ts` is a setup
  file that makes every `fetch` reject, so no web test reaches a backend (#387).
- **Tooling:** `bin/lib/*.test.sh` for every hook and `bin/` script.
- Never `cargo test --test-threads=1` for the full suite: threads sharing one `cargo test` process
  collide in the sqlx harness, and serializing them to avoid it is slower than nextest's process per
  test. `just test-rust` stays as a human convenience.

## Reading a test result (MANDATORY)

Every rule that says "all tests must pass" depends on you being able to tell. **The exit code
cannot tell you, and it is wrong in BOTH directions.** Background-task summaries have reported
"exit code 0" for logs that ended in a failure, and a step that never ran exits as cleanly as one
that passed.

**Read the summary line. Nothing else is evidence.**

- nextest: the `Summary` line (`N tests run: N passed, M skipped`), and the `FAIL` lines it repeats
  beneath it.
- `cargo test` (doc tests, `just test-rust`): every `test result:` line, one per binary.
- vitest: the `Test Files` and `Tests` lines, per package.
- `just test` / `just ci-full`: the per-step summary at the end. A step whose tool is missing reports
  MISSING, which is not PASS and blocks the marker.

**A collapsed test count means the run ABORTED, not that it failed.** If `tests run` has dropped
sharply against your baseline, a test binary failed to build or list, or a filter excluded more than
you meant. Find out which before you believe anything else the run says.

**Name the failing test before you reason about it.** You can only do that if you kept the log, so
never pipe a suite run through `tail`: the summary survives the tail window, but the failing test's
name and panic message are far above it. Redirect the whole run to a file in your scratchpad and
grep that file afterwards (`sed -n '/^failures:$/,/^$/p'` for `cargo test` output).

**turbo can replay a cached log, another worktree's included.** `pnpm test`, `pnpm typecheck` and
`pnpm lint` may print `cache hit, replaying logs`: turbo's cache is shared across worktrees, and its
key ignores root files the tasks read (`DESIGN.md`, `desktop/src-tauri/tauri.conf.json`,
`eslint.config.mjs`), so a replay once stamped a red vitest commit green. `bin/run-tests.sh` and
`bin/pre-push` run `pnpm exec turbo run <task> --force`. By hand, do the same and confirm
`cache bypass, force executing`, or run `pnpm --filter <pkg> exec vitest run` in the package.

## A tier you did not run is not a tier that passed

The section above is about misreading a summary line. This one is about there being no summary
line at all, which is easier to wave through because nothing on screen contradicts you. A tier
that produced no output produced no evidence, and "no failures" is not a finding about the code
until you have watched that harness succeed at something.

**Prove the harness is live before you trust its silence.** Run one test you expect to pass and
watch it pass. Then the absence of failures elsewhere means something. Skip that and a harness
which cannot start at all is indistinguishable from a clean run. On #457 `just ci` was green with
589 tests passing while CI's clippy job failed on the branch's own defect: `just ci` never runs
clippy.

The running stack is where this bites, because its failures do not look like infrastructure. A
backend that refuses to boot because the shared database holds another branch's migrations
("previously applied but is missing") answers every request with a connection error, the same
shape as a broken endpoint. **Read the backend's log before diagnosing anything.** A wall of
failures with one boot error behind them is one environment problem, not thirty defects.

**Say which tiers ran, in the report and on the PR.** "Tests pass" that silently means "the tiers I
could run pass" is the claim this rule exists to stop. Name the tier you skipped and why: CI's
cross-platform `desktop` matrix and any `windows-latest` job never run locally, and the full gate
reports a tool it cannot find (`cargo-nextest`) as MISSING. The audits are ADVISORY there: a
missing `cargo-deny` shows as not run, and CI's `deny` job still runs it.

## Blast-radius test tiering — right-size the baseline

Scale the suite you run to what the change can **reach**, not which directory it lives in. Tier
up when unsure.

| Tier | Trigger: what the diff can reach | Run |
| --- | --- | --- |
| **Full** | the trajectory/ETA model and every caller, the feed subsystem, metering/TMIs, permissions/auth/scope, the API contract, migrations, `jobs.rs` passes, the outbound-jobs → bot path, a web page or route | `just test` **AND** the changed page or endpoint exercised on the running stack (`just backend` + `just web`) against a throwaway database |
| **Standard** | a handler, repo or job with no web surface, no contract change and no migration | `just test`; the running stack only for the touched endpoints |
| **Light** | tests-only, tooling, docs, comments, `.claude/**` | targeted runs for the touched area (below, and `bash bin/lib/<name>.test.sh`); **no stack walk** |

"Tier on what it can reach" is the whole point: a one-line change in `feed/predict.rs` reaches every
ETA surface and is **Full**, however small the diff. A diff with no runtime surface (a test file, a
rule doc) gets no stack walk.

**One exception that overrides the tier: shared test infrastructure.** A change to
`backend/src/scope_test_support.rs`, `web/src/test/no-network.ts`, `.config/nextest.toml` or a
vitest setup file touches no production code, but every test that uses it does. A filtered run
cannot tell you whether you broke the rest. Run the full suite for those.

Before starting work, validate the baseline is green (or note the known pre-existing failures)
so you never attribute existing red to your own diff. See "Don't auto-fix existing pre-existing
test failures unrelated to your branch" below.

**Diagnose with targeted runs; confirm with the full one.** A full `just test` compiles and runs
every crate and package, so it is a confirmation instrument, not a debugger. Re-running it to see
whether an intermittent failure fires again is gambling, and it charges the user a full build a
roll. When a test fails in a full run and passes alone, the next command is the smallest one that
can reproduce it:

```bash
cargo nextest run -p <crate> -E 'test(<name>)'     # one Rust test, or a module with a prefix
pnpm --filter <pkg> exec vitest run <path>          # one web test file
bash bin/run-tests.sh rust -p ois-backend -E 'test(scope_)'   # the same, through the gate script
bash bin/run-tests.sh web web src/lib/time.test.ts            # (never stamps)
```

After that it is reading the code for shared state, not another full run. Spend the full run once
you have a fix to confirm.

## A gate counts once per SHA

Rust compiles dominate the cost of every gate, so a full gate runs once per commit and its result
is reused wherever it is needed.

- **`just test` is the full local gate** (`bin/run-tests.sh` with no arguments): everything
  `just ci-full` runs, in one go. It stamps `.claude/markers/tests/<sha>` and its manifest line only
  for a green run on a clean, committed HEAD that did not move during the run. With arguments,
  `bin/run-tests.sh` passes them through to a targeted run and never stamps. Writing a marker
  yourself is prohibited (CLAUDE.md → "Absolute Prohibitions (enforced by hooks)").
- **A gate counts once per SHA, wherever it ran.** For the exact SHA under review, either of these
  is the full gate: a tests marker in this checkout, or green CI check-runs on that SHA, every
  check-run concluded `success` (the check-runs are the job list, never a copied one):
  ```bash
  gh api repos/VATUSA/OIS/commits/<sha>/check-runs --jq '.check_runs[] | "\(.conclusion) \(.name)"'
  ```
  An empty list straight after a push means CI has not started, not green. Run `just test` only
  when neither exists, and never re-run the full suite on a SHA that already has one. Targeted runs
  (a mutation's one test, an isolated failure) are always fine.
- **Use nextest for the full Rust suite**, as CI does, never `cargo test -- --test-threads=1`.

## Running the gate while other worktrees are live

Worktrees are meant to run in parallel. They still share one machine, one Postgres, the sccache
cache and turbo's cache, and that produces failure shapes that look like real ones.

- **Each worktree has its own `target/`** (seeded from the primary checkout's at setup, sources
  touched so workspace crates rebuild). Never share one `CARGO_TARGET_DIR` across worktrees: cargo
  judges path crates fresh by mtime, so a worktree runs another's stale test binary and passes.
- **Full Rust suites queue on Postgres.** `bin/run-tests.sh` runs the full nextest suite under
  `bin/with-lock.sh postgres`, so two worktrees' full suites never share the server; a waiter prints
  who it is waiting on. A targeted run, clippy and the JS suite run unlocked. A bare
  `cargo nextest run --workspace` bypasses the lock.
- **Starvation looks like a hang.** Before deciding a run is stuck, check
  `pgrep -fl 'cargo|nextest'`, `pgrep rustc | wc -l` and `uptime`.
- **The `#[sqlx::test]` harness flakes under contention.** The panic sits inside `sqlx-core`
  (`database "_sqlx_test_…" does not exist`, `PoolTimedOut`, `already exists`), never in an
  assertion, and it lands on a different unrelated test each run. Re-run each named failure on its
  own: a flake does not survive isolation; a real failure repeats and sits in code you touched. Say
  in the PR which failures were flakes and that you re-ran them.

**Never run two full suites in one checkout at once; the pre-push hook counts as one**
(`bin/pre-push` runs clippy and the suite unless `just test` stamped the pushed commit). At most one
full `just test` of your own at a time.

### A missing `.env` is thousands of errors, none of them yours

`.env` is gitignored and `node_modules` is per worktree, so a fresh worktree fails in shapes that
look like a broken branch. No `.env`: every `#[sqlx::test]` fails with `DATABASE_URL must be set`.
No `node_modules`: the JS half dies with `turbo: command not found` after the Rust half passed.
Before believing a mass failure, run `bin/verify-worktree.sh`. A failure unrelated to the change
(missing env, a database that will not accept connections, a tool not on `PATH`) is environmental
until proven otherwise.

### A failure in a file you did not touch is someone else writing in your checkout

Several agents work this repo at once, and other sessions have edited and even deleted worktrees
mid-gate. A compile error in an unrelated file, a tracked file you did not modify, or a stray
`.bak` is the tell. Check `git status --porcelain` before and after every run, and re-run once the
other session's work is out of the tree.

## Green proves nothing on its own

A passing test proves the assertion ran. It does not prove the assertion could ever fail. The
Five Questions (`test-writing.md`) are a thought experiment; this is the experiment: **break the
code and watch the test go red.** Delete the guard, invert the condition, remove the predicate:
run the test, confirm it fails for the reason you expect, then restore. If it stays green, the
test is decoration.

Do this whenever a test defends a subtle invariant: a permission or ARTCC-scope check, a
destructive statement, a time window, a dedup key, a tuning constant, a trajectory or metering
formula. On a flaky test it is mandatory: the test was green before you touched it too.

**Then revert the actual bug and run the suite.** In OIS the regression is usually a call site
drifting, not the unit misbehaving: five returns (#428, #433 twice, #419) had a well-tested unit and
an untested caller. If the suite stays green with the real bug back in, the tests do not defend the
fix (`test-writing.md` → "Test the wiring, not just the unit").

**Restore the mutation in a way that survives the run being killed.** A restore placed after the
test command, on its own line, does not run when the command is killed, and the mutation stays on
disk where the next thing you do is commit it. `bin/mutate.sh FILE OLD NEW -- COMMAND...` applies
one mutation, runs one command, prints `RED` or `GREEN` and restores the bytes on every exit path.
Around it:

- **Assert the mutation applied.** `git diff --quiet` after the edit means it did not; re-check the
  pattern after `cargo fmt` reflows lines.
- **Never mutate while a suite builds in the same tree.** cargo and vitest read sources at build
  time, so a background run picks up your mutant (#699). Mutate first, or in a second worktree.
- **One verification run per worktree.** To abandon a backgrounded script, stop the task itself,
  not its test binary: killing the child lets the shell run its next mutation (#585).
- **Commit a WIP checkpoint before mutating by hand**, through the hooks, never `--no-verify`: the
  pre-existing-failure exception does not cover it; if a hook blocks it, copy the files to your
  scratchpad instead.
  `git checkout -- <file>` restores HEAD, not "before the mutation", and has deleted uncommitted
  tests mid-cycle (#472).
- **Mutate `.claude/hooks/` only in a scratch copy**: every session runs the hooks from the primary
  checkout.

**Assert the invariant, not a weaker proxy for it.** A test that checks a required step is
*present* but not its *position* still passes when someone reorders it; a test meant to prove an
effect reaches *many* rows but written `assert!(!rows.is_empty())` still passes on a single-row
fixture; a test named for a boundary that only asserts "non-empty" never exercises the boundary.
Assert the exact property the behavior depends on (the count, the ordering, the boundary value)
so a plausible mutation makes it red.

**A test that reimplements the code's own arithmetic has tested the arithmetic twice.** Reading a
constant in a test is ordinary; recomputing the formula under test from it is not, because then
both sides move together and nothing can fail. Take one side from the code and the other from the
requirement: drive the real function and hold its output against a literal. A test that
recomputes the ISA Mach↔TAS conversion or a descent schedule to check `feed/trajectory.rs` cannot
detect a change to either.

**When the point of the change IS a constant, a test must pin that constant.** A fixture built from
the constant moves with it: backdating a row by `DEPARTURE_RUNWAY_RETAIN_HOURS + 1` stayed green
when the retention went from 12 h to 8,760 h (#509). Use absolute fixtures that straddle the
documented value (11 h and 13 h) plus an assert that the constant still sits between them, then put
the old value back and watch that test, and only that test, go red.

The `unwrap_or(default)` fallbacks on an env var or a config row are the same trap one level down.
They fire only when the value is missing, which is the one case no test sets up, so they carry a
stale default indefinitely. Pin them too, or make a missing value fail loudly.

**Exercise a scaling factor at something other than 1.** A multiplier only ever tested at its
identity value is indistinguishable from no multiplier: delete `* factor` and every test stays
green. Pin at least one case at a non-identity value with an exact expected output.

**Drive every entry in a list the code matches against, not just the first.** A filter, allowlist
or denylist is only proven for the entries a test actually pushes through it. An entry with a typo,
or one the matching logic cannot reach, reads as protection and is inert. Loop the cases in the test
and name the failing entry in the assertion message, so a dead entry fails on its own rather than
hiding behind a live sibling.

**Inject the clock before asserting on a delay, a window or any `now`-derived offset.** The value is
computed from `Utc::now()` when the code runs and compared against `Utc::now()` again when the
assertion runs, so any stall between the two shifts every offset. The OIS pattern is a
`now: DateTime<Utc>` parameter the test passes; where a `jobs.rs` pass reads the clock itself,
anchor fixtures on `Utc::now()`, never a calendar date, and keep them clear of truncation edges
(`num_minutes()` truncates).

**Read the test's own name as a checklist, and assert every clause of it.** A name of the form
"X does not happen when Y" makes two promises and is routinely written asserting only that Y
worked. The half a name promises and the body omits is almost always the negative half: no row
written, no job enqueued in `integration.outbound_jobs`, no request sent. When a name says
"nothing", "only", "does not" or "without", find the assertion that would fail if the thing did
happen; if there is none, the name describes coverage you have not written.

**Every "it must NOT appear" assertion needs a positive control beside it.** A negative assertion
passes for two very different reasons: the guard worked, or the setup never got the subject into
the state you meant to test. Assert something in the same response that proves the mechanism was
live. A `403` from a scope check is equally true of a request that never reached the handler, so
the same test sends one that succeeds.

**When a change moves a rule onto a different signal, test both directions they can disagree in,
and find out which one real data takes.** Two signals for one fact can disagree two ways: the new
one says yes where the old said no, or no where the old said yes. One direction makes the rule
fire more, the other makes it fire less. A test that drives only one of them passes against a
predicate that still honors the old signal in the other, so the change can revert without a red
test. In OIS the standing cases are a national-scope and a facility-scope principal, and a flight
whose live and historical paths read different sources.

- **Count the directions against real data before writing the test.** A `GROUP BY` over the shared
  dev database, read-only, settles it in one query. Quote when that data was captured beside the
  count.
- **Pin the direction that occurs, with a case that fires beside it.** The direction real data
  takes needs its own test, next to a control that does fire.
- **Prove it by reinstating the old signal.** OR the old input back into the predicate. The test
  that pins the shipped direction must go red, not only the test for the direction nobody hits.

**A shell probe that disagrees with the code is usually the probe.** Two specific traps make a
verification command lie silently, and both read as a finding rather than a mistake:

- `echo "... $(grep -c 'unwrap($e)' file)"`: the outer double quotes expand `$e` to nothing
  *before* grep sees it, so the pattern becomes `unwrap()` and matches the wrong thing. Use single
  quotes on the outer string, or a heredoc.
- `FILES=$(...); grep pattern $FILES`: zsh does not word-split, so a newline-separated file list
  arrives as one enormous filename. Write the list to a file and use `xargs`.

A probe returning nothing is evidence about the probe. These are the two shapes that produce a
*confident wrong number* instead of nothing, which is harder to catch. When a probe contradicts a
test result, believe the test.

**Test a parser against bytes captured from the real producer, never input you typed**
(`test-writing.md` → "Test a parser against captured bytes, never against input you typed").

## jsdom vs the running stack — the decision rule

OIS has no browser test suite. Browser-facing verification is two things, and a web page or
component with **genuine browser behavior** gets both: a vitest DOM test for the component's
behavior, AND a run of the real page against the running stack (`just backend` + `just web`) on a
throwaway database (`database-postgres.md`), with the URL and what you saw recorded in the report.
Never invent a browser-test layer that does not exist.

**The decision rule: put each check in the cheapest layer that can actually see it.** These do not
need a DOM or a running stack:

- **Pure logic in a component.** Extract it to `web/src/lib/*` and test it in the node environment
  (`command-scopes.ts` and its test are the pattern).
- **Authorization and visibility scoping.** See/don't-see by permission, 401/403/404 sweeps, IDOR:
  that is the backend's job, proven with a router-level request (`test-writing.md` → "A handler
  call cannot prove an authorization gate").
- **Server-side values.** Anything computed in a handler, repo or the feed (an ETA, a metering
  slot, a demand bucket) is a Rust test of the computation plus a router test of what the handler
  returns.

**A jsdom test is for interaction**: keyboard handlers, click propagation, `aria-pressed`, a
query-cache state the component renders. `renderToStaticMarkup` cannot render anything that
portals (`Modal` uses `createPortal`), so an interaction test mounts with `createRoot` in jsdom.

**Keep it for the running stack ONLY for a concrete dependency jsdom lacks, and name it in the
report:** layout and CSS geometry (jsdom has no layout engine), MapLibre/WebGL rendering, real
VATSIM OAuth and cookie handling, CORS and CSP, the realtime socket on `GET /api/v1/ws`, and the
Tauri webview. A page whose every check is reproducible in jsdom gets no stack walk beyond loading
it once.

## Don't auto-fix existing pre-existing test failures unrelated to your branch

When you run the full suite and see N failures, FIRST establish whether they exist on
`origin/next`: green CI check-runs on `origin/next`'s head SHA settle it without a build, or run the
failing tests by name from the primary checkout, which sits on `next`. Never `git stash` to compare;
a stash in a session that gets interrupted strands the work (#499). Pre-existing failures are not
regressions from your branch. Report them separately in the review summary.
