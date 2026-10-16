---
description: Ship the current issue — gate, commit, review the commit, push, open the PR against next, Moment 3, and move the card to Post build.
argument-hint: "[issue number]"
---

You are wrapping up the current work and shipping it. Follow these steps in order — do NOT skip any.

The order is deliberate: **commit first, then review the committed commit, then push and open the PR.** The review marker is keyed to a commit SHA (Step 4), so the thing you review is the exact thing that ships. Reviewing a dirty worktree and committing afterwards means you shipped a commit nobody reviewed as a unit.

## Step 1: Run the test suite (scaled to blast radius)

1. Tier the run to what the change can **reach**, per `.claude/rules/test-quality.md` → "Blast-radius test tiering — right-size the baseline" — not by which directory the diff sits in. A one-line change in a function the trajectory model calls is **Full**; a docs / tooling / `.claude/**` change is **Light** (no stack walk).
2. If the API contract moved (an endpoint, or a `#[derive(ToSchema)]` model or its `///` doc comments), **regenerate the client first** (`AGENTS.md` § The API contract → typed client).
3. Run the tier as targeted runs: `cargo nextest run -p <crate> -E '<filter>'` for each crate the change reaches, `pnpm --filter <pkg> exec vitest run` for each package, `pnpm typecheck` after a contract change. Do NOT run `just test` here: the tree is not committed, so it stamps nothing, and `/review-before-shipping` runs it once on the committed HEAD, where it writes the tests marker. **Read the summary line, not the exit code** — it is wrong in both directions (see `.claude/rules/test-quality.md` → "Reading a test result"). All tests must pass.
4. Walk the change on the running stack (from the worktree, against a throwaway database) when the diff has a **browser surface** — any page, component, route or endpoint the web app calls — or for a Full-tier change, and record each URL and what you saw. Skip it only when the diff has no browser surface at all, and say so in the PR.
5. If any tests fail, fix and re-run until green. Do NOT proceed until green.

## Step 2: Inspect for test coverage gaps

1. Run `git diff origin/next...HEAD --stat` to see all changed files. Never the local `next` ref, and always three dots: two dots renders everything merged into `next` since you forked as a deletion.
2. For each changed source file, verify test coverage exists:
   - New handlers, repos, jobs and functions have corresponding tests
   - Edge cases are covered (null values, missing data, authorization, ARTCC scope)
   - Error paths are tested
   - A permission or scope gate has a real-router test (`scope_test_support::send`), a DB behavior a `#[sqlx::test]`, a component with behavior a vitest DOM test
3. Write any missing tests (`/write-tests` helps). Re-run the affected tests after adding them.

## Step 3: Commit

Committing to the feature branch is NOT "shipping" — the push and PR are what wait for sign-off, not the local commit. Review needs a committed SHA to key its marker to (Step 4), so commit first.

1. Verify branch name matches `{feat|fix|chore}/{issue-or-hash}/{2-4-word-description}` (max 50 chars) — never `worktree-*`, which `/start` should already have renamed. **NEVER commit onto `next` or `main` — you work in the issue's worktree** (see CLAUDE.md "Absolute Prohibitions (enforced by hooks)"). If you are on `next` or `main`, stop and ask.
2. Stage changed files with **explicit paths** (never `git add -A` / `git add .` / `git add --all`), then commit: conventional `type(scope): summary`, a short body, `Closes #<n>`, and no trailer or attribution of any kind, even when the session suggests one (`AGENTS.md` § Git workflow).
3. **Post-commit diff-integrity check (MANDATORY).** After the commit succeeds, run:
   ```bash
   git diff origin/next...HEAD --name-only
   git status --short
   ```
   The first must list **every** file you intended to ship and nothing else. The second must NOT show any of those files as still-modified (`M`). A pre-commit refusal (`rustfmt --check`, gitleaks, migration versions) leaves nothing committed, and a `just fmt` run after staging leaves its fixes unstaged — if anything is missing or still shows `M`, stage and commit the rest before moving on.
4. **Attribution check over every commit the PR will carry.** Two plain commands (no git inside `$(...)`), the file in your scratchpad:
   ```bash
   git log --format='%an <%ae>%n%cn <%ce>%n%B' origin/next..HEAD > <scratchpad>/commits.txt
   bash -c '. <primary-checkout>/.claude/hooks/lib/attribution.sh; attribution_hits <scratchpad>/commits.txt; echo "rc=$?"'
   ```
   `rc=1` is clean; `rc=0` printed the offending lines (reword those commits); `rc=2` means the check did not run. Every author and committer line is the user.

## Step 4: Code review (against the committed HEAD)

Run `/review-before-shipping` — follow all phases to completion. It reviews the current committed HEAD and, on passing, writes the **SHA-keyed** markers `.claude/markers/review-shipping/<HEAD-sha>` (checked by `pre-pr-gate.sh`) and `.claude/markers/code-review/<HEAD-sha>`. It covers the issue cross-check, the gates in one `just test` run (fmt, clippy, nextest, doc tests, lint, typecheck, vitest, the audits, client drift), the pre-commit checklist, and `/code-review`'s six lens reviews with their attestations and pitfall scan.

**If the review applies a fix,** that fix is a new commit → HEAD moves → the markers are for the old SHA. Commit the fix, re-run the Step 3 diff-integrity check, and re-run `/review-before-shipping` so it stamps the new HEAD. Never copy a marker onto a new SHA, and never write one here: the marker is keyed to the commit it reviewed, and "never bypass tooling" applies.

## Step 5: Verify the reviewed commit, then push

1. **Ship the reviewed commit as-is.** HEAD must be the SHA `/review-before-shipping` reviewed, and `git status --porcelain` must print nothing. Anything changed since the review means re-running `/review-before-shipping`; never commit it here.
2. **Confirm the markers, the attestation and the gate for HEAD:** `.claude/markers/review-shipping/<sha>` and `.claude/markers/code-review/<sha>` exist, and `bash bin/lib/review-attestation.sh check` prints `attestation complete`. If any is missing, the review did not pass for this commit: re-run it. The gate counts once per SHA, wherever it ran (`test-quality.md` → "A gate counts once per SHA"), so either proof satisfies it: `.claude/markers/tests/<sha>` in this checkout, or green CI check-runs on that exact SHA (`gh api repos/VATUSA/OIS/commits/<sha>/check-runs --jq '.check_runs[] | "\(.conclusion) \(.name)"'`, every job concluded `success`). With neither, run `just test`. This holds on rework too, where the PR already exists and `pre-pr-gate.sh` does not fire.
3. **A new migration:** re-scan the migration numbers on every remote branch immediately before pushing and chain the push on the scan (`database-postgres.md`); a duplicate half-migrates the database at startup.
4. `git push -u origin {branch-name}`. The pre-push hook skips clippy and the suite when a tests marker in this checkout proves them; with green CI as the only proof it runs them.
5. Verify the push against the remote itself, not the local tracking ref:
   ```bash
   git ls-remote origin refs/heads/{branch-name}
   git rev-parse HEAD
   ```
   The two SHAs must match. A push has failed silently before; nothing after this step happens until they do. Chain later steps on the push with `&&`, never `;`.

## Step 6: Create PR

PR creation is gated by `pre-pr-gate.sh`, which requires the `review-shipping` marker and a complete lens attestation for the PR's head commit, and `test-run-gate.sh`, which requires the tests marker. Write the body for a junior developer, with the before/after diagram `.claude/rules/engineering-prose.md` → "What a PR description must do" asks for, and put the tests lens's guard inventory in it. Create the PR against `next`:

```bash
gh pr create --repo VATUSA/OIS --base next --title "type(scope): summary" --body-file <scratchpad>/pr.md
```

with `pr.md` holding:

```markdown
#<n> [short summary] (<board column>)

Closes #<n>

## Summary
<what is being fixed, in ordinary words, before any type or function name>

## How it works
<fenced before/after data-flow diagram>

## Test plan
- <grounded checklist of what actually ran on this branch: the tier and its summary lines, `just test` on <sha>, pages and endpoints walked with their URLs>
- <every gate you did NOT run, and why; any deviation from the ACs, stated plainly>
```

No attribution line of any kind; check the file with `! grep -qiE 'Drafted by|Generated with|Co-Authored' <scratchpad>/pr.md` before creating. **Read the PR number from `gh pr create`'s output** (the `/pull/<n>` URL it prints). Never predict it: concurrent sessions take "the next number". If the branch is stacked on another open PR, say so on the body's first line.

**Rework on an open PR:** skip `gh pr create`; the Step 5 push already updated the PR. Read its number with `gh pr view {branch} --repo VATUSA/OIS --json number --jq .number`. Once a branch is pushed, bring in `origin/next` by merging, never rebasing.

OIS **has** CI (`.github/workflows/ci.yml`), and it runs checks `just test` can't (the cross-platform `desktop` matrix, any `windows-latest` job). Read the PR head's check-runs: `gh api repos/VATUSA/OIS/commits/<sha>/check-runs --jq '.check_runs[] | "\(.conclusion) \(.name)"'`. An empty list right after a push means CI hasn't started, not green. Report them as they stand, pending included; don't poll in a loop (the API budget is shared with the board). Under the ticket loop, `ticket-worker.md` → Phase D waits for them.

**Under the ticket loop, post the worker's evidence comment on the PR straight after it opens, before the `Post build` move** (`ticket-worker.md` → "Review it as ticket-reviewer will"): the AC walk, the mutation table against the tests lens's guard inventory, the untouched paths, claims with `file:line`, and "Operator decisions and rejected findings". That section is the issue's decisions record (`code-review.md` → Phase 3 "Operator decisions"): copy `<git common dir>/review-decisions/<key>.md` into it, confirm the comment posted, then delete the file. From here the PR comment is the record. Outside the ticket loop there is no evidence comment and the file stays the record.

## Step 7: Moment 3 — verification comment and board move

**First, re-fetch the issue body's `## Acceptance criteria` section and compare it with lens `ac`'s attestation for HEAD** (`.claude/markers/attest/<HEAD>/ac.md`): same count, same text. Any difference means an AC changed since the review: **STOP** and re-run `/review-before-shipping`.

Follow `.claude/rules/github-issues.md` → "Moment 3". Before posting, cut every line of the draft that names test machinery (nextest, vitest, clippy, `just test`, suite results, mutation proofs), a source or test file path, branch, rebase or worktree mechanics, a note addressed to a code reviewer rather than to whoever verifies the behavior, or an incidental tidy-up: those belong in the PR. Post the verification comment with its length and attribution asserted as a gate, the body file in your scratchpad:

```bash
python3 -c 'import sys; n = len(open(sys.argv[1]).read().rstrip()); print(n); sys.exit(n > 1200)' <scratchpad>/moment3.md \
  && ! grep -qiE 'Drafted by|Generated with|Co-Authored' <scratchpad>/moment3.md \
  && gh issue comment <n> --repo VATUSA/OIS --body-file <scratchpad>/moment3.md
```

A skipped `gh` call is not a posted comment: read the issue back. Then move the board item to **`Post build`**:

```bash
bin/board-status.sh <n> "Post build"
gh issue view <n> --repo VATUSA/OIS --json projectItems,assignees \
  --jq '{status: [.projectItems[] | select(.title == "OIS Kanban") | .status.name], assignees: [.assignees[].login]}'
```

Not `Code Review`, `Testing Queue` or `Done` — those are the user's. Read the status back before reporting the move, and confirm the issue is assigned to the user.

## Step 8: Tear down the worktree

The branch is on `origin` and the PR is open, so the local worktree and its throwaway database are disposable. Remove all of it. Review feedback later is resumed with `/start <n>`, which rebuilds a worktree from the pushed branch.

1. **Confirm nothing exists only locally**, from inside the worktree. All three must hold; if any does not, STOP and report it rather than tearing down.

   ```bash
   git status --porcelain                          # must print nothing
   git rev-parse --abbrev-ref HEAD                 # the branch
   git rev-parse HEAD                              # must equal the next line
   git ls-remote origin refs/heads/<branch>
   ```

   Separate plain commands, values written literally: inside a worktree session the harness refuses git nested in `$(...)`.

   Record the worktree directory name (`basename "$PWD"`) and the branch name for sub-step 3.

2. **Run `ExitWorktree` with `action: "remove"` and `discard_changes: true`.** The tool refuses without `discard_changes` whenever the branch has commits not on the default branch, which a shipped branch always does; sub-step 1 proved those commits are on `origin`, so nothing is discarded. Its PreToolUse hook (`.claude/hooks/worktree-teardown.sh`) runs `bin/teardown-worktree.sh`, which drops the worktree's throwaway database, removes the directory, and deletes the local branch. The harness holds a lock on the worktree while the session is inside it; teardown recognizes the exiting session's own lock and overrides it. If the hook blocks, read its message, fix the cause, and retry; never remove the directory by hand.

   If `ExitWorktree` reports no active worktree session (the worktree came from an earlier session), run the teardown directly from the primary checkout instead: `bash bin/teardown-worktree.sh <dir-name>`.

3. **Verify from the primary checkout** — evidence, not the tool's message:

   ```bash
   ls -d .claude/worktrees/<dir-name> 2>/dev/null && echo "DIR STILL PRESENT" || echo "dir removed"
   git branch --list "<branch>"                     # empty
   git worktree list                                # not listed
   git rev-parse --abbrev-ref HEAD                  # next
   git pull --ff-only origin next
   ```

   And the throwaway database `bin/setup-worktree.sh` reported for this worktree is gone. Anything left over: run `bash bin/teardown-worktree.sh <dir-name>` (idempotent) and re-check. The remote branch and the PR are deliberately left alone.
