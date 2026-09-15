---
description: Ship the current issue — gate, commit, review the commit, push, open the PR, move the card, and hand back.
argument-hint: "[issue number]"
---

You are wrapping up and shipping the current issue (`$ARGUMENTS`). Order is deliberate — **commit
first, then review the commit, then push and open the PR** — so the thing reviewed is the exact thing
that ships. Do NOT skip steps.

## Step 1 — Gate (scaled to blast radius)
Run `just ci` (fmt-check, cargo check/clippy, rust tests, pnpm lint/typecheck). If the API contract
moved, **regenerate the client first** (see `/review-before-shipping` Phase 3). Read the real
`test result:` / typecheck output, not the exit code. Green before proceeding — fix and re-run
otherwise.

## Step 2 — Coverage check
`git diff next...HEAD --stat`. For each changed source file, confirm new logic has a test and sad
paths are covered (metering / trajectory / permission resolution especially). Add missing tests and
re-gate.

## Step 3 — Commit (to the feature branch, never `next`)
- Confirm the branch is `{feat|fix|chore}/{issue}/{desc}` and you are in the issue's worktree — **never
  commit onto `next`**; if you are on `next`, stop and ask.
- Stage with **explicit paths** (not `git add -A`), then commit. Message: conventional
  `type(scope): summary`, a short body, `Closes #$ARGUMENTS`. **No attribution trailer** — commits in
  this repo are authored solely as the user; never add `Co-Authored-By`, a session link, or any
  agent/AI credit (see `CLAUDE.md`).
- Integrity check: `git status --short` (nothing you meant to ship still shows `M`) and
  `git diff origin/next..HEAD --name-only` (lists every intended file).

## Step 4 — Review the committed HEAD
Run `/review-before-shipping` to completion against this commit. If it applies a fix, commit it,
re-run the Step 3 integrity check, and re-run the review so it covers the new HEAD.

## Step 5 — Push
`git push -u origin {branch}`; verify with `git branch -vv` (no unpushed commits).

## Step 6 — Open the PR
```bash
gh pr create --repo VATUSA/OIS --base next --title "type(scope): summary" --body "$(cat <<'EOF'
## Summary
<1–3 bullets — what changed and why>

## Test plan
- <grounded checklist of what you actually verified: `just ci`, endpoints exercised, UI checked>
EOF
)"
```
OIS **has** CI (`.github/workflows/ci.yml`) — `gh pr checks` returns real checks; confirm they go
green, but `just ci` locally is the primary evidence. Do not sit blocked waiting on the remote run.

## Step 7 — Move the card + Moment 3 comment
- `.claude/scripts/board-status.sh $ARGUMENTS "Code Review"` and confirm the issue is **assigned to
  me**.
- Post the **Moment 3** comment on the issue (≤1,200 chars): real **file paths** only; name the
  **blast radius** (does it touch the trajectory/ETA model, the permission/role three-in-sync
  invariants, or the OpenAPI→client contract — or none); and the **deploy note** — any new migration
  (applies on backend startup, sequential number) and whether the generated client must be
  regenerated. If the change is entirely `docs/` or **test-only** (`#[cfg(test)]` / web tests), say
  so with justification (not application logic, not data-affecting) so it can skip runtime verification.

## Step 8 — Hand back
Return to the `next` worktree, `git pull`, and remove the issue worktree if finished
(`git worktree remove ../ois-wt/{branch}`). Refresh your inventory from the **board** (not memory)
before the next issue.
