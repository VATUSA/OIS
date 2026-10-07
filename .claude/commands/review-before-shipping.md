---
description: Formal review of the committed HEAD before shipping — gates, pitfall scan, three fresh review agents, fixes, and the SHA-keyed review marker.
---

You are performing a formal review before shipping. Review the **committed HEAD** of the feature
branch (commit first — a dirty worktree means uncommitted changes were never reviewed). Work through
every phase in order. The review marker in Phase 6 is the only output `/ship` can rely on: the
`pre-pr-gate.sh` hook blocks `gh pr create` for any HEAD without one.

Under `/ticket-loop` this runs inside the `ticket-worker` subagent: every "ask me" below becomes an
`OPERATOR_QUESTIONS` entry in its report, not a guess.

## Phase 0 — Pin the reviewed commit

- If the API contract moved (an endpoint, or a `#[derive(ToSchema)]` model or its `///` doc
  comments), **regenerate the client first** (`AGENTS.md` § The API contract → typed client) and
  commit it, so the pinned commit already carries it.
- `git status --porcelain` must print nothing. If it prints anything, commit it (or remove it)
  first; the review is of a commit, not a tree.
- Pin HEAD, the full 40-character **reviewed SHA**, with the snippet below. Every phase reviews that
  commit, and Phase 6 writes the marker for the pinned SHA only while HEAD still equals it. The pin
  lives in this worktree's own git dir, so parallel worktrees never share one.
- If a marker for this exact SHA already exists and is under two hours old (Phase 6 shows where),
  the review is already valid for this commit. Say so and stop; don't re-run the pipeline on a
  byte-identical commit.

```bash
bash -c 'set -euo pipefail
[[ -z "$(git status --porcelain --untracked-files=no)" ]] || { echo "BLOCKER: uncommitted changes; commit first"; exit 1; }
pin="$(git rev-parse --path-format=absolute --git-dir)/ois-reviewed-sha"
git rev-parse HEAD >"$pin" && echo "reviewed sha: $(cat "$pin")"'
```

## Phase 1 — Diff inventory

`git diff origin/next...HEAD --stat` (three dots: two dots renders everything merged into `next`
since you forked as a deletion), then read the full diff. Know exactly what was added, changed and
removed, and which of these it reaches: the trajectory/ETA model (`backend/src/feed/trajectory.rs`
and every caller `git grep -n 'trajectory::'` lists), the permission/role three-in-sync invariants,
the OpenAPI→client contract, a migration, the feed subsystem.

## Phase 2 — Gates

- Run **`just ci-full`**. It mirrors `.github/workflows/ci.yml` (fmt, clippy `-D warnings`, nextest or
  `cargo test`, doc tests, pnpm lint/typecheck/test, the audits, cargo deny, client drift) and ends
  with a PASS/FAIL/SKIPPED summary.
- Read the real `test result:` / typecheck lines, not the exit code. Every step must PASS; a SKIPPED
  step is named in the report and later in the PR's test plan, never counted as passed.
- Iterating on one failure, run just that step. For Rust tests that is `just test-rust`
  (`cargo test --workspace --all-targets -- --test-threads=1`), not a bare `cargo test --workspace`,
  which drops both flags and matches neither the local recipe nor CI's nextest run. Re-run
  `just ci-full` once it is fixed.
- A red step you can show is pre-existing (it fails identically on `origin/next`) is reported as
  such, with the evidence, not fixed here.
- **Any commit made to get a gate green moves HEAD**: re-pin (Phase 0) and restart from Phase 1, so
  the reviewers and the marker see the same commit.

## Phase 3 — Pitfall scan

```bash
.claude/scripts/review-scan.sh origin/next
```

It scans `origin/next...HEAD` in HEAD's tree for the OIS mistakes that compile and pass tests:
broken permission or role three-in-sync, a handler in `router.rs` but not `openapi.rs` (or the
reverse), SQL or `unwrap()`/`expect()` in `handlers/`, an applied migration edited in place, a
mutating route without `RequirePermission<P>`. It prints `file:line: message`, exits 0 when clean, 1
on findings, and 2 when it could not run — exit 2 is a failed phase, never a clean one.

Each hit is a prompt to look, not proof. Confirm it against the diff, then carry real ones into
Phase 5 as CRITICAL.

## Phase 4 — Three fresh reviewers, in parallel

Dispatch all three **in one message** as fresh subagents, so they run concurrently and none shares
this session's assumptions. Give each only the branch name, the reviewed SHA, the base
(`origin/next`), and the issue number. No summary of what you built, and no opinion on it.

| Agent | Looks for | Blocks on |
| --- | --- | --- |
| `code-review-agent` | correctness, contract drift, sync-invariants, missing checks | CRITICAL, WARNING |
| `security-audit-agent` | authz gaps, IDOR, SQL built from input, secret leaks, graded G1–G7 | G1–G4 |
| `test-reviewer` | tests that pass without proving anything; missing sad paths | CRITICAL, WARNING |

Wait for all three. Nothing terminal (a fix commit, the marker, a report) happens while one is still
running. Act on each agent's first report; don't keep one alive chasing the tail of a truncated list.

## Phase 5 — Consolidate and fix

Merge the three reports and the Phase 3 hits into one list, deduplicated by `file:line`, graded:

- **CRITICAL** — code-review/test-reviewer CRITICAL, security G1–G2, a confirmed scan hit.
- **MAJOR** — code-review/test-reviewer WARNING, security G3–G4.
- **Minor** — SUGGESTION, NIT, G5–G7. Listed in the report, not acted on. Size and complexity numbers
  are review guidance only: they are a SUGGESTION, never a blocker.

Before fixing anything, try to disprove it: read the code the finding names and trace the path. A
finding that dies under that is listed as dismissed, with the reason. Dismissing a CRITICAL or MAJOR
is my call, not yours: ask me (under `/ticket-loop`, an `OPERATOR_QUESTIONS` entry) with your
evidence before the marker is written.

**Prove the tests can fail.** `test-reviewer` is read-only, so it describes mutations rather than
running them. Run each one it describes; mutate every case `test-quality.md` makes mandatory that
the diff touches (a permission or scope check, a destructive statement, a time window, a dedup key, a
tuning constant); and for a fix branch revert the actual bug. Follow
`.claude/rules/test-quality.md` § Prove the test can fail (commit a checkpoint first; confirm the
mutation applied; never mutate while a suite is building in the same tree). A test that stays green is
a MAJOR. Record each mutation and whether it went red for the Phase 7 report.

Fix every surviving CRITICAL and MAJOR **as a new commit** (never amend the reviewed commit).
**Ask me before**: changing a test's expected behavior; altering business logic or a calculation;
creating or editing a migration; removing or renaming a public API or permission; anything
user-visible. **Auto-fix without asking**: typos, formatting, obvious missing error handling, doc
comments, a missing test for behavior the code already has.

After the fix commits:

1. Stage explicit paths, commit, and run the integrity check (`git status --short` is clean,
   `git diff origin/next...HEAD --name-only` lists every intended file).
2. **Re-pin** (the Phase 0 snippet): the new HEAD is now the reviewed SHA.
3. Re-run Phase 2 and Phase 3 on it.
4. Re-dispatch fresh instances of whichever agents raised the fixed findings, scoped to the fix range
   `<old reviewed SHA>...HEAD` plus their original findings: does each fix resolve the finding,
   introduce nothing new, and leave everything else as it was? Loop until none raises a CRITICAL or
   MAJOR.

## Phase 6 — Write the review marker

Only when Phases 2–5 are clean. Run it as written: it reads the SHA Phase 0 pinned, so there is
nothing to fill in.

```bash
bash -c 'set -euo pipefail
sha="$(cat "$(git rev-parse --path-format=absolute --git-dir)/ois-reviewed-sha" 2>/dev/null)" || { echo "BLOCKER: nothing pinned; run Phase 0"; exit 1; }
[[ "$sha" =~ ^[0-9a-f]{40}$ ]] || { echo "BLOCKER: not a full 40-character sha: $sha"; exit 1; }
[[ "$(git rev-parse HEAD)" == "$sha" ]] || { echo "BLOCKER: HEAD moved off $sha; review the new HEAD"; exit 1; }
[[ -z "$(git status --porcelain --untracked-files=no)" ]] || { echo "BLOCKER: uncommitted changes; commit first"; exit 1; }
dir="$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")/.claude/markers/review-shipping"
mkdir -p "$dir" && date +%s >"$dir/$sha" && echo "review marker: $dir/$sha"'
```

The marker lives under the **primary checkout's** `.claude` (resolved through the git common dir),
so `pre-pr-gate.sh` finds it from any worktree. Its name is the commit SHA and its content the unix
time it was written: any new commit moves HEAD off it and re-blocks the PR, and it goes stale after
two hours. Never copy a marker onto another SHA or write one for a review that did not run.

## Phase 7 — Report and stop

Report: the reviewed SHA; `just ci-full`'s summary (every SKIPPED step named); the scan result; each
agent's verdict line; each mutation and whether it went red; what you fixed (with the fix commits);
minor findings left as they are; and anything dismissed and why.

`/review-before-shipping` ends here. It does not push, open a PR, comment on the issue, or move the
card; that is `/ship`.
