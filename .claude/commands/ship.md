---
description: Ship the current issue — gate, commit, review the commit, push, open the PR, move the card, and hand back.
argument-hint: "[issue number]"
---

You are wrapping up and shipping the current issue (`$ARGUMENTS`). Order is deliberate — **commit
first, then review the commit, then push and open the PR** — so the thing reviewed is the exact thing
that ships. Do NOT skip steps.

Two hooks enforce the order (`.claude/settings.json`): `attribution-gate.sh` blocks a commit or PR
carrying AI attribution, and `pre-pr-gate.sh` blocks `gh pr create` unless
`/review-before-shipping` wrote a marker for the current HEAD. A blocked call is the process
working; fix the cause, never work around the hook.

## Step 1 — Gate
Run **`just ci-full`**: everything `.github/workflows/ci.yml` runs that can run on this machine, with a
PASS/FAIL/SKIPPED line per step (`AGENTS.md` § Commands). If the API contract moved, **regenerate the
client first** (`AGENTS.md` § The API contract → typed client). Read the real `test result:` /
typecheck output, not the exit code. Every step PASS before proceeding; note each SKIPPED step for the
PR's test plan.

## Step 2 — Coverage check
`git diff origin/next...HEAD --stat`. For each changed source file, confirm new logic has a test and
sad paths are covered (metering / trajectory / permission resolution especially). Add missing tests
(`/write-tests` helps) and re-gate.

## Step 3 — Commit (to the feature branch, never `next`)
- Confirm the branch is `{feat|fix|chore}/{issue}/{desc}` and you are in the issue's worktree — **never
  commit onto `next`**; if you are on `next`, stop and ask.
- Stage with **explicit paths** (not `git add -A`), then commit. Message: conventional
  `type(scope): summary`, a short body, `Closes #$ARGUMENTS`. **No attribution trailer** — commits in
  this repo are authored solely as the user; never add `Co-Authored-By`, a session link, or any
  agent/AI credit, even when the session suggests one (see `CLAUDE.md`).
- Integrity check: `git status --short` (nothing you meant to ship still shows `M`) and
  `git diff origin/next...HEAD --name-only` (lists every intended file, and nothing else).
- Attribution check over every commit the PR will carry, using the same patterns the hooks use. It
  must print `attribution: clean`:
  ```bash
  bash -c 'set -uo pipefail
  . "$(git rev-parse --show-toplevel)/.claude/hooks/lib/attribution.sh" || { echo "BLOCKER: attribution lib not found"; exit 2; }
  log="$(git log --format="%an <%ae>%n%cn <%ce>%n%B" origin/next..HEAD)" || { echo "BLOCKER: git log failed"; exit 2; }
  printf "%s\n" "$log" | attribution_hits -; rc=$?
  case $rc in 0) echo "BLOCKER: attribution found"; exit 1 ;; 1) echo "attribution: clean" ;; *) echo "BLOCKER: check did not run"; exit 2 ;; esac'
  ```
  Also read `git log --format='%an <%ae>' origin/next..HEAD | sort -u`: the only author is the user.

## Step 4 — Review the committed HEAD
Run `/review-before-shipping` to completion against this commit. It runs the scan and the three
fresh review agents, fixes CRITICAL/MAJOR findings as new commits, and writes the SHA-keyed marker
for the final HEAD. If you commit anything after it finished, HEAD moved: run it again.

## Step 5 — Push, and prove it
```bash
git push -u origin {branch}
git ls-remote origin refs/heads/{branch}   # must print exactly the SHA below
git rev-parse HEAD
```
Compare the two SHAs yourself. **Rework on an open PR:** a push updates the PR directly and
`pre-pr-gate.sh` only guards PR creation, so before pushing confirm the marker for HEAD exists (the
path `/review-before-shipping` Phase 6 printed); no marker, no push. A push has failed silently before (`.claude/rules/git-and-worktrees.md`
§ A push isn't done until the remote says so); nothing after this step happens until they match.
Chain later steps on the push with `&&`, never `;`.

## Step 6 — Open the PR
```bash
gh pr create --repo VATUSA/OIS --base next --title "type(scope): summary" --body "$(cat <<'EOF'
## Summary
<1–3 bullets — what changed and why, readable by someone who wasn't here>

Closes #<issue>

## Test plan
- <grounded checklist of what you actually ran: `just ci-full` with its summary, endpoints exercised, UI checked>
- <every gate you did NOT run, and every SKIPPED step, by name>
EOF
)"
```
`pre-pr-gate.sh` refuses this unless HEAD carries a fresh review marker; if it blocks, go back to
Step 4. **Rework on an open PR:** skip `gh pr create`; the Step 5 push already updated the PR. **Read the PR number from `gh`'s output** (the `/pull/<n>` URL it prints). Never predict it:
concurrent sessions take "the next number". If the branch is stacked on another open PR, say so on the
body's first line.

OIS **has** CI (`.github/workflows/ci.yml`), and it runs checks `just ci-full` can't (the desktop
matrix). Before you report a gate green, read the PR's check-runs once (`gh pr checks <n>`) and report
what they say, pending included. Don't sit blocked on the remote run, and don't poll it in a loop: it
shares the API budget with the board.

## Step 7 — Move the card + Moment 3 comment
- `.claude/scripts/board-status.sh $ARGUMENTS "Testing Queue"`, read the card back, and confirm the
  issue is **assigned to me**. Never `Code Review`, `Shippable` or `Done`; those are a human's. (The
  one exception: under `/ticket-loop`, `ticket-reviewer` moves a card to `Code Review` when the
  operator's own pass decision is relayed to it.)
- Post the **Moment 3** comment on the issue: at most **1,200 characters** of body text, the footer
  excluded. Real **file paths** only; name the **blast radius** (trajectory/ETA model, the
  permission/role three-in-sync invariants, the OpenAPI→client contract, or none); and the **deploy
  note** — any new migration (applies on backend startup, sequential number) and whether the
  generated client must be regenerated. If the change is entirely `docs/`, tooling, or **test-only**
  (`#[cfg(test)]` / web tests), say so with justification (not application logic, not data-affecting)
  so it can skip runtime verification. Assert the length before posting:
  ```bash
  n=$(LC_ALL=en_US.UTF-8 wc -m < moment3.md | tr -d ' ')
  [ "$n" -le 1200 ] && { cat moment3.md; printf '\n\n🤖 Drafted by Claude Code\n'; } > moment3-post.md &&
    gh issue comment $ARGUMENTS --repo VATUSA/OIS --body-file moment3-post.md
  ```
  Keep both files in your scratchpad, not the worktree. If the assertion fails, shorten the body; a
  skipped `gh` call is not a posted comment, so read the issue back.

## Step 8 — Hand back
Report what changed against the ACs, which gates ran and which did not, and what a tester should
check. Then return to the primary checkout and `git pull` there. When you are finished with the
worktree, remove it with `/cleanup` (it refuses unless every commit is on `origin`); leave it when the
task says to keep it for review. Refresh your inventory from the **board** (not memory) before the
next issue.
