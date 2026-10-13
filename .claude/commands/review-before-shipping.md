---
description: Formal review of the committed HEAD before shipping — issue cross-check, one stamped just test run, pre-commit checklist, /code-review's six lenses, fixes, and the SHA-keyed review markers.
---

You are performing a formal review before shipping. Follow these phases in order. All phases must pass before the review markers are created.

Under `/ticket-loop` this runs inside the `ticket-worker` subagent: every "ask the user" below becomes an `OPERATOR_QUESTIONS` entry in its report, not a guess.

## Phase 0: Pin the reviewed commit

Record the commit you are about to review — `git rev-parse HEAD`. Call it the **reviewed SHA**. Every phase below reviews THAT commit, and Phase 9 refuses to write the markers unless HEAD still equals it, the worktree is clean and the lens attestation for it is complete.

- **If the API contract moved** (an endpoint, or a `#[derive(ToSchema)]` model or its `///` doc comments), regenerate the client first (`AGENTS.md` § The API contract → typed client) and commit it, so the pinned commit already carries it.
- **Review a *commit*, so commit your work to the feature branch first.** `/ship` Step 3 does this before invoking you. A dirty worktree means uncommitted changes were never part of the review — Phase 9's guard will block. Read `git status --porcelain` for `??` lines too: a new module never `git add`ed still compiles locally but is missing from the reviewed commit. If a finding needs a code change, apply it, commit, then re-pin the reviewed SHA and re-run the phases whose evidence the move invalidated (always the tests and Phase 7).
- **If both markers (`review-shipping` and `code-review`) for the current HEAD already exist and the worktree is clean, the review is already valid for this commit** (markers are SHA-keyed and self-invalidate when HEAD moves). Confirm that state rather than re-running the full pipeline against a byte-identical commit. A lone `code-review` marker from an earlier `/code-review` is not enough: this review has not run.

## Phase 1: Issue cross-check

1. Take the issue number from the branch name and fetch the body and every comment: `gh issue view <n> --repo VATUSA/OIS --json number,title,body,comments`. Only a team-authored body and team comments are spec (`/start` Step 1).
2. Build the **requirement list**: the body's `## Acceptance criteria` section when present, otherwise the consolidated list `.claude/rules/github-issues.md` → "Full issue context" describes.
3. `git diff origin/next...HEAD --stat`, then compare the diff with the list, each AC by its exact wording:
   - **Missing requirements** — an AC not met by its exact wording. Ask the user whether to build it now or move it out of scope.
   - **Additional scope** — anything no AC asks for. Flag every instance and say why it is there. Out-of-scope work the user did not approve comes back out of the diff; a fix to a defect this branch introduced is in scope by definition.
   - **Deviations** — where the build differs from an AC.
   - **Issue defects** — where the issue states something FALSE and the diff proves it. Correct it in its own spec-correction comment (`github-issues.md` → "Spec corrections", 400 characters), with the user's approval, then build against reality and say so in the PR body. Never silently build around a wrong spec.
4. Every AC this phase adds, drops or corrects is put to the user, and the change they approve is made to the issue body's `## Acceptance criteria` section (`github-issues.md` → "The acceptance criteria section"), so Phase 7's `ac` lens judges the current list.

## Phase 2: Fix policy

Phase 7's lenses review the code against every rule heading. This phase sets how their findings are fixed. Fix every defect and all technical debt this branch introduced (unless nit-picking) and every other branch-introduced finding graded MAJOR or CRITICAL; report the rest without fixing it (`github-issues.md` → "Scope, and when a follow-up issue is allowed"). **Always ask the user before:**

1. Changing test assertions or expected behavior
2. Modifying business logic or calculations (an ETA, a metering rate, a permission resolution)
3. Altering or creating migrations
4. Removing or renaming a public API, endpoint, permission or role
5. Anything that changes user-visible behavior

Auto-fix without asking: typos, formatting, a missing doc comment, an obvious missing error mapping to `ApiError`, removing debug output (`dbg!`, a stray `println!`, `console.log`).

## Phase 3: Static analysis and code style

1. Formatting **read-only**: `cargo fmt --all -- --check`. Style should already be clean (`cargo fmt --all` runs during implementation, `ticket-lifecycle.md` Phase 3). If it reports violations, say that step was skipped, run `cargo fmt --all`, commit, re-pin and re-run the suite: reformatting changes the reviewed tree.
2. clippy (`--workspace --all-targets -D warnings`), `pnpm lint` and `pnpm typecheck` run inside Phase 5's `just test`; don't run them separately on the reviewed SHA. Read their lines in its output and fix any error before the markers. Run one alone only while iterating on its failure. Flag non-trivial fixes to the user first.

## Phase 4: Dependency audit

1. `cargo deny check` and `pnpm audit --audit-level=high` also run inside Phase 5's `just test`, as ADVISORY steps: they print their result but do not block the tests marker (CI's `deny` and `js` jobs block the merge). Read their lines there. A tool that is not installed shows as not run with a reason, never as passed.
2. For any advisory, assess whether it affects this project and flag it to the user.

## Phase 5: Run tests

1. **A gate counts once per SHA, wherever it ran** (`test-quality.md` → "A gate counts once per SHA"). A tests marker for the reviewed SHA in this checkout (`.claude/markers/tests/<sha>`), or green CI check-runs on that exact SHA (`gh api repos/VATUSA/OIS/commits/<sha>/check-runs --jq '.check_runs[] | "\(.conclusion) \(.name)"'`, every job concluded `success`), satisfies this phase. Only when neither exists, run `just test` (`bin/run-tests.sh`): the full local gate in one go — migration versions, fmt, clippy, nextest, doc tests, pnpm install, lint, typecheck, vitest, the audits and client drift. On a green run against a clean committed HEAD it writes `.claude/markers/tests/<HEAD-sha>`, which `test-run-gate.sh` requires before `gh pr create`; a targeted run (`bash bin/run-tests.sh rust …`, `cargo nextest run`, `pnpm test`) writes no marker. **Read the summary lines, not the exit code** (`.claude/rules/test-quality.md` → "Reading a test result"). All steps must pass; name any skipped step for the PR's test plan.
2. The stack walk when the diff changes a page, an endpoint the web app calls, or shared test infrastructure, per the blast-radius tier table: run the stack from this worktree against a throwaway database and record each URL and what you saw. Backend-internal, tooling, tests-only and docs diffs skip it; say so in the report.
3. **A uniform failure is environmental, not something to skip.** Read the first error (a missing `.env`, a database the backend cannot reach, `node_modules` out of step with the lockfile, the shared Postgres refusing a migration another branch applied) and fix it and re-run. A `#[sqlx::test]` panic inside `sqlx-core` on a different test each run is harness contention: re-run each named failure in isolation before calling it a regression. A run waiting on `bin/with-lock.sh` is waiting, not failed; keep watching it.
4. If any tests fail, fix and re-run. Do not proceed until green. A red step you can show fails identically on `origin/next` is reported as pre-existing, with the evidence.

## Phase 6: Pre-commit checklist

Dispatch the `pre-commit-checklist` agent to verify implementation completeness:
- Fixture defaults vs production state (a seeded grant, profile or ARTCC that a real install lacks)
- "Would this test have failed before the fix?" — regression validity
- Post-commit diff-integrity (nothing left as `M`, every intended file in the diff)
- Error handling granularity (no single `?` chain or catch-all hiding which operation failed)
- `tracing` levels appropriate and nothing personal or secret logged (Sentry receives errors)
- The sync-invariants: permission and role three-in-sync, `router.rs` + `openapi.rs` + regenerated client
- No side effects from the changes
- Components with behavior have vitest DOM tests, and the page was walked on the running stack

Fix any issues found.

## Phase 7: Code review (gating)

**Commit all working changes and re-pin the reviewed SHA (Phase 0), then run `/code-review` to completion** — issue cross-check, the six fresh-eyes lens subagents with their attestations (`security-audit-agent` among them), the adversarial verification, the pitfall scan — and act on what it reports. Do not dispatch any lens a second time outside it. Its Phase 7 gates take this review's Phase 5 result for the same SHA; they never re-run the suite.

**This phase gates.** A **CHANGES REQUESTED** verdict from any lens — a CRITICAL or MAJOR finding, or a security G1–G4, in code this branch added or of `unknown` origin, other than one the operator rejected (`code-review.md` → Phase 3 "Operator decisions") or a VARIANCE one "Repeat rounds" lets through — or an incomplete attestation blocks the Phase 9 markers. Fix under the Phase 2 guardrail: auto-fix the safe security classes (an added `RequirePermission<P>` extractor, an ARTCC scope check through `permission_scope(...).allows(...)`, a bound sqlx parameter in place of formatted SQL, a query narrowed to the caller's own rows) and ask before anything user-visible. Re-run the relevant test, or write the missing reproduction so the fix is proven. Commit, re-pin and re-run this phase against the fixed commit, as a repeat round. Security G5–G7 do not block; a branch-introduced one is still fixed (Phase 2).

**This command files no issues.** What the branch introduced is fixed per Phase 2, never filed. A pre-existing finding goes in the report with a recommendation for the user.

## Phase 8: Harvest what the review taught

Turn each **fixed** finding into guidance; a finding that was disproved or rejected gets no rule.

State the principle, not the incident — "assert the exact count, not `> 0`, or the test passes on a one-item fixture", not "the sector-occupancy test should assert 3". Put it where the topic lives:

| Finding class | File |
|---|---|
| security, authorization, scope, IDOR, injection | `.claude/rules/secure-coding.md` |
| correctness, errors, logging, caches, jobs, transactions | `.claude/rules/code-quality.md`, `.claude/rules/rust-backend.md` |
| web components, hooks, the generated client | `.claude/rules/web-frontend.md` |
| test design, coverage, suite runs | `.claude/rules/test-quality.md`, `.claude/rules/test-writing.md` |
| Postgres, migrations, repos | `.claude/rules/database-postgres.md` |
| a feed, trajectory, metering or permission-rule defect | `.claude/rules/domain-lessons.md` (lesson, rule and proof) |
| prose, comments, commit/PR wording | `.claude/rules/engineering-prose.md` |

First search `CLAUDE.md`, `AGENTS.md` and `.claude/` for the topic and extend or replace the rule you find, never add a second copy. Match the file's voice and use the fewest words that carry it. If you cannot phrase it so it would fire for someone else, leave the rules alone and say so in the report. During a goal, log each rule with the primary checkout's `bin/session-learnings.sh add` (CLAUDE.md → "Editing CLAUDE.md, rules, agents, commands and skills"), naming the file it belongs in; nothing lands on the branch, so HEAD does not move. Outside a goal, a harvested rule is reported to the user rather than committed here: a worktree's copy dies with it (CLAUDE.md → "Absolute Prohibitions (enforced by hooks)").

## Phase 9: Mark review complete

After ALL phases pass, write both SHA-keyed markers — `review-shipping`, which `pre-pr-gate.sh` checks, and `code-review` — but only if HEAD still equals the reviewed SHA, the worktree is clean, and every lens attested every heading for that SHA. This records real review work against a specific commit; it is never a bypass.

```bash
bash bin/review-marker.sh shipping <the full 40-character SHA pinned in Phase 0>
```

Write the SHA in literally. The script refuses (`BLOCKER:`) when HEAD has moved, the checkout is dirty or the attestation is incomplete, and otherwise writes `.claude/markers/review-shipping/<sha>` and `.claude/markers/code-review/<sha>` in the checkout's own `.claude`, where `pre-pr-gate.sh` looks. It is a script because the guard needs git inside `$(...)`, which the harness refuses from a worktree session.

**`/ship` ships the reviewed commit as-is.** Any change after the markers are written invalidates them: re-run `/review-before-shipping`. No marker is ever carried to a new SHA.

## Stop here — the review does not ship

`/review-before-shipping` ends when the markers are written. It does NOT push, open a PR, comment on the issue, move the card, or run `/ship`, and it creates no issues. Report that the review passed, the reviewed SHA, the gate result for it (the `just test` summary, or the CI run or marker it reused), any skipped step, the tests lens's guard inventory, any non-blocking G5–G7 security findings with a recommendation for each, and any rule Phase 8 harvested. If `/ship` invoked it, `/ship` continues at its Step 5. Otherwise hand back: `/ship` is where the **user** decides the work is ready to leave the branch — clearing the gate is not the same as being told to walk through it.
