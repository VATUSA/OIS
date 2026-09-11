---
description: Begin an OIS issue — read the thread, claim it on the board, set up a worktree, validate the baseline, and plan.
argument-hint: "[issue number]"
---

You are starting work on an OIS issue. `$ARGUMENTS` is the issue number (if empty, list issues in
**To Do** or **Returned** on the board that are assigned to me and ask which with AskUserQuestion —
never pick one yourself). Everything goes through `gh` against `VATUSA/OIS`. Do NOT skip steps.

## 1. Read the whole thread

- `gh issue view $ARGUMENTS --repo VATUSA/OIS --comments`.
- Confirm it is **assigned to me** and sits in **To Do** or **Returned**. If it is not assigned to
  me, stop — do not touch it. If it is **Returned**, understand *why* from the comments first.
- The body **and every comment** are the spec; a later comment **overrides** the body. Read all of it.
- If it carries **`technical-debt`** and doesn't actually fix or improve the operability of the
  system, present your findings, explain why, and let me decide whether to abandon (AskUserQuestion).
- Extract the **acceptance criteria**, present them, and let me confirm which to fulfil. (AI-drafted
  issues cause bloat and requirements poisoning — do not accept them uncritically.)

## 2. Confirm and claim

- Confirm with me whether to work it now — let me answer **yes / no / skip** (AskUserQuestion).
- On **yes**, immediately claim it so a concurrent agent doesn't:
  `.claude/scripts/board-status.sh $ARGUMENTS "In build"`.

## 3. Recover or set up the worktree

- **Recover existing work first** — `git worktree list` and `git branch --list "*/$ARGUMENTS/*"`.
  If a branch/worktree for this issue exists, resume it; do **not** repeat work already done.
- Otherwise create a **temporary worktree** and branch. The "work on `next`, never create or switch
  branches" rule protects the primary checkout; a worktree is the sanctioned exception. Fork from
  `next` (the integration branch; PRs target it, and `main` is promoted from it separately — see #87):
  ```bash
  git worktree add ../ois-wt/{branch} -b {branch} origin/next
  ```
  Branch format: `{type}/{issue}/{2-4-word-desc}`, max 50 chars, `type` ∈ `feat|fix|chore`
  (e.g. `feat/47/save-event-replays`). Work only inside that worktree.

## 4. Validate the baseline

- Run the gate scaled to what the change can **reach**, not where it sits — a change touching the
  **trajectory/ETA model**, the **permission/role three-in-sync invariants**, or the
  **OpenAPI→client contract** reaches further than its directory. `just ci` is the full gate.
- Read the actual `test result:` / typecheck output — **never the exit code**, which lies in both
  directions. Classify any red as **pre-existing** or **introduced** before you write anything.

## 5. Plan (mandatory)

- Enter **PLAN MODE**. Present the plan for my approval **before writing any code**.
- The plan must state the **acceptance criteria** and link the issue (`VATUSA/OIS#$ARGUMENTS`).
- Stay strictly within the issue's scope (body, comments, review feedback) — do not expand scope or
  over-engineer; produce the simplest winning solution.
- Let me guide in plan mode: propose options, work with me, and keep the **ACs + issue number
  visible in every prompt**.
