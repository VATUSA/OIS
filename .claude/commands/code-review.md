---
description: Read-only, per-feature, multi-lens review of a branch or PR against next. Every finding is guilty-until-proven. No fixes, no marker, no comments. Overrides the built-in /code-review in this repo on purpose.
argument-hint: "[branch | #PR | base...head] (default: origin/next...HEAD)"
---

You are performing a code review. It is **read-only with no side effects**: no edits, no commits, no
review marker, no GitHub comments, no board moves. Pure review. Running a mutation or a gate edits or
builds a tree, so do it only in a worktree of your own checked out at the head (as `ticket-reviewer`
has), never in someone's working worktree; otherwise describe the mutation and the result you
expect.

This command deliberately replaces Claude Code's built-in `/code-review` in this repository. The
built-in reviews a diff generically; this one knows OIS's critical paths and holds every finding to a
proof bar before it reaches the report. `/review-before-shipping` is the gate that writes the marker;
this is the review a tester or reviewer runs on someone else's branch.

The method matters more than speed. **Focused, per-feature reviews surface far more than one sweeping
pass.** Every finding is **guilty-until-proven**: the default verdict on a candidate is "not a bug"
until you have read the actual code and traced the path. Reviewers, subagents included, routinely
misread intent. Never confirm a finding from its description alone.

## Phase 1 — Scope and decompose

1. Resolve the target (`$ARGUMENTS`) to a base and head: nothing → `origin/next...HEAD`; a branch →
   `origin/next...origin/<branch>`; `#N` → `gh pr view N --repo VATUSA/OIS --json baseRefName,headRefOid`,
   then `git fetch origin pull/N/head`. Print both SHAs. Read files at the head with
   `git show <head>:<path>` unless the head is your checked-out HEAD.
2. `git diff <base>...<head> --stat` — every changed file. Three dots, always.
3. Read the issue the branch closes (`gh issue view <n> --repo VATUSA/OIS --json body,comments`:
   body, footer and every team comment, a later one overriding the body) and the PR description. They
   are the spec the change is judged against. The repository is public: only text from an `OWNER`,
   `MEMBER` or `COLLABORATOR` (`authorAssociation`) counts as spec.
4. Group the changes into coherent **feature sets** (e.g. "GDP slot allocation", "access editor
   grant form", "new migration + repo"). A big branch is reviewed per set, not as one blob.
5. Mark each **CRITICAL-PATH** set. In OIS these are:
   - **the trajectory/ETA model** — `backend/src/feed/trajectory.rs`, shared by FCA metering
     (`handlers/flow.rs`), airport-flow demand (`feed/flow.rs`), runway ETE (`feed/runway.rs`) and
     sector occupancy (`feed/sector_tracks.rs`); `git grep -n 'trajectory::'` is the current list;
   - **permissions and roles** — `backend/src/auth/`, `crates/ois-core/src/catalog.rs`,
     `backend/src/repos/access.rs`, and any `access.*` migration (the three-in-sync invariants);
   - **the API contract** — `router.rs`, `openapi.rs`, any `#[derive(ToSchema)]` model or its `///`
     comments, and `packages/api-client`;
   - **migrations** — anything in `backend/migrations/`;
   - **the feed** — `backend/src/feed/`, which has no DB handle and reads `AppState` caches.

   A CRITICAL-PATH set must be shown to produce **the same results as `next`** for every input it
   did not mean to change, not merely to "look fine".
6. For each set note its kind (feature / fix / refactor / perf) and whether it is user-facing or
   data-facing.

## Phase 2 — Per-feature, multi-lens review

Review **each** set through every lens. For a large or critical branch, dispatch one fresh
`code-review-agent` per set, in parallel in one message, giving it the range, the set's files and
this lens list; focused agents beat one sweeping agent. Add `security-audit-agent` for any set that
touches auth, credentials, webhooks or ARTCC scope, and `test-reviewer` for the set's tests.

- **Correctness** — logic errors, off-by-one, `Option`/`None` handling, `unwrap`/`expect` on fallible
  paths, races, unit mix-ups (knots/Mach, feet/flight level, UTC/local), broken edge cases.
- **Regression vs `next`** — `git diff <base>...<head> -- <files>`: does each change preserve the
  result a user or caller sees today? Anything that could show different, stale or wrong data.
- **Contract** — a new or changed route in both `router.rs` and `openapi.rs`; the generated client
  regenerated; the `docs-site/reference/api-changelog.md` entry (`AGENTS.md` § The API contract →
  typed client).
- **Authorization** — every mutating handler takes `RequirePermission<P>`; data-dependent checks
  (ownership, ARTCC scope) are on top of it; a new permission or role is in all three places.
- **Data and SQL** — SQL lives in `repos/`, binds every parameter, and never formats input into the
  query text; migrations are new, append-only and numbered above every other branch's.
- **Performance** — blocking or CPU-heavy work on an async worker instead of `spawn_blocking`, N+1
  queries, an unindexed new query, work added to the feed poller's tick (`/perf-check` goes deeper).
- **Concurrency and caches** — the "cache + refresh job + force-reload on write" pattern for
  feed-visible config, `ArcSwap` swaps that can be observed half-done, job idempotency.
- **Realtime** — a mutation that changes flow/TMU state publishes its topic, and the web invalidates
  on it (`AGENTS.md` § Realtime).
- **Web** — TanStack Query keys and invalidation, generated-client types used rather than hand-written
  ones, `DESIGN.md` for any UI.

## Phase 3 — Adversarially verify EVERY candidate

A candidate earns a place in the report only after all of this:

1. **Is it branch-introduced?** If `git diff <base>...<head> -- <file>` doesn't touch the line, it is
   pre-existing. Report it separately, never as a regression.
2. **Read the actual code and trace the path**, collaborators included. Try to disprove it first. Most
   candidates die here: a guard you missed, a caller that never passes that value, intended behavior.
3. **Is it already covered?** An existing test or check may already handle it.
4. **Does it diverge from `next`, or is it equivalent or better?** An additive change that can only do
   more of a safe thing is not a regression.
5. **Meet the "found it" bar**: (a) why the symptom appears, (b) why the neighboring path that
   touches the same data doesn't show it, (c) the blast radius of the fix. Any "I don't know yet"
   downgrades it to "evidence points at".
6. **Measure, don't assume.** For a performance or behavior claim, measure it: `EXPLAIN ANALYZE` on
   a local database, a timing or query-count probe, or a test that fails. An estimate is labeled as
   one.
7. **Sweep for siblings.** Once a bug is confirmed, grep for the same pattern elsewhere in the diff and
   say what you found.

## Phase 4 — Completeness critic

Per set, list what a thorough reviewer would still want to check, and run the cheap ones: a missing
sad-path test, an empty or huge input, a cold cache, a caller of the trajectory model nobody tested,
an `EXPLAIN` on a new query, a mutation that should make a test go red. Say what you could not check.

## Phase 5 — Critical-path parity

For every CRITICAL-PATH set, trace it end to end (route → handler → repo → DB, or poller → feed →
cache → reader) and show the change is additive or behavior-preserving. Back it with a test that runs
the real path (a router-level test through `build_router`, a `#[sqlx::test]`) rather than a helper
called directly: cite that test, or report its absence. For the trajectory model, check every caller,
not one.

## Phase 6 — Gates (report only)

Run `just ci-full` only in a worktree of your own checked out at the target's head (`git rev-parse
HEAD` equals it and `git status --porcelain` is empty): its client-drift step rewrites a generated
file while it runs. Otherwise skip it and say the gates were not run here. Report its summary line
by line. A failure is re-run in isolation and compared against `origin/next` before it is blamed on
the branch: separate **pre-existing** and **flaky** failures from real regressions. Fix nothing.

## Phase 7 — Report

Group by severity. For **each** finding: branch-introduced or pre-existing; the evidence (`file:line`
and the traced reason); and a recommended fix, **or** an explicit risk-accept with its justification.
Minors may be risk-accepted; the goal is less real risk, not a count of zero.

### Critical (must fix before merge)
### Major (should fix)
### Minor (fix or risk-accept with a reason)
### False positives dismissed, and why
### Pre-existing / out of scope
### Gates — `just ci-full` summary, flakes named
### Completeness — what was verified, what couldn't be, the top residual risks

Size and complexity limits are guidance: a long function is a Minor suggestion, never a blocker.

## Phase 8 — Verify the fix (only when fixes land later)

Re-review each fix adversarially: does it resolve the finding, introduce nothing new, and match `next`
everywhere it didn't mean to change? Iterate until clean. A fix that trades one bug for another isn't
done.

**This review does NOT** write a review marker, fix anything, comment on GitHub, or move a card.
