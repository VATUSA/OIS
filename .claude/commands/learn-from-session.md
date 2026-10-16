---
description: Review this session for the classes of problems and inefficiencies it hit, and fold the lessons into this repository's Claude files (CLAUDE.md, .claude/rules, agents, commands, skills). Never touches user-level ~/.claude files.
---

Review this session and improve this repository's guidance so the next session avoids the same classes of problem.

## 1. Find the classes, not the incidents

List what cost time, caused rework or nearly shipped a defect: wrong assumptions, slips, retries, detours, misread tools or specs, checks that passed vacuously, work redone after review. For each, name the **class** it belongs to and why it happened: the mechanism, not the issue, file, type name or command that was involved.

- Bad: "`update_advisory` must not clear the caller's `decoded`."
- Good: "A fix copied from a sibling inherits the sibling's assumptions; diff the two request types before accepting it."

Drop anything that happened once for a reason that will not recur, and anything existing guidance already covered but was not followed; the fix there is following it, not repeating it.

A resolved feed, trajectory, metering or permission-rule defect is the exception to "classes, not incidents": it goes in `.claude/rules/domain-lessons.md` as lesson, rule and proof (a commit and a test that goes red without the fix).

## 2. Prefer judgment over rules

For each class, decide the lightest guidance that would have changed the outcome:

1. **Nothing**: a capable engineer would get it right next time without being told.
2. **Sharpen an existing principle**: widen or clarify a rule already there so it covers the class.
3. **A new principle**: one that states the underlying reasoning, so it applies to cases nobody has met yet.

Write principles that explain *why*, so the reader can judge new cases. Do not write checklists of specific names, paths or incidents; they go stale and only catch the exact repeat.

## 3. Propose, then apply

Show every proposed change to the user with `AskUserQuestion` before editing: the file, the exact text, and the class it addresses. Apply only what is approved.

Write each change per `CLAUDE.md` → "Editing CLAUDE.md, rules, agents, commands and skills": fewest words that carry it; search `CLAUDE.md`, `AGENTS.md` and `.claude/` for the topic first and extend or replace what is there, never add a second copy. A fact about the codebase (architecture, commands, env, testing mechanics) belongs in `AGENTS.md`; process belongs in `CLAUDE.md` and `.claude/`.

## Scope: repository files only

- **Edit only this repository's `CLAUDE.md`, `AGENTS.md` and files under `.claude/`** (rules, agents, commands, skills). Never create, modify or delete anything under `~/.claude/`, including the user-level `CLAUDE.md`, memory and user commands. If a lesson belongs there, say so in your report and leave it to the user.
- **During a goal, log each approved change, never edit**: `bash bin/session-learnings.sh add <file> <context> <learning> <fix>`, the goal's end applying the file (`CLAUDE.md` → "Editing CLAUDE.md, rules, agents, commands and skills").
- **Outside a goal, never write to a feature worktree's copy** (it dies with the worktree, `ticket-lifecycle.md` → "Worktrees") **or to the primary checkout**, which stays on `next`. Do not create or switch branches and do not commit on your own: list the approved edits in your report and ask the user for a branch. On a yes, apply them on a `chore/{short-hash}/<desc>` worktree forked from `origin/next`, as `/ticket-loop` → "6. END" does.
