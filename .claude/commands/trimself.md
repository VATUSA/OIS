---
description: Trim CLAUDE.md, rules, agents, commands and skills to the editing directive, resolve conflicting rules, and prove no instruction was lost
---

Trim the checked-in guidance so it follows `CLAUDE.md` → "Editing CLAUDE.md, rules, agents, commands and skills".

Scope: `CLAUDE.md`, `.claude/rules/*.md`, `.claude/agents/*.md`, `.claude/commands/*.md`, `.claude/skills/**/*.md`. `AGENTS.md` is the facts file: trim its wording only when the user includes it, and never move a fact out of it into a rule. Project files only; never anything under `~/.claude/`.

1. **Branch.** Work in a worktree on a `chore/{short-hash}/trim-rules` branch forked from `origin/next` (`EnterWorktree`, then the branch fix and provisioning `/start` Step 2 gives), never in the primary checkout, which stays on `next`. Creating it needs the user's go-ahead: you do not create branches on your own. Record the base commit: `BASE=$(git rev-parse HEAD)`.
2. **Measure.** Record `wc -l` for every file in scope, and `wc -m` for the always-loaded set: `CLAUDE.md` plus every `.claude/rules/*.md` without `paths:` frontmatter. Budget: 130,000 characters. If trimming alone cannot meet it, ask via `AskUserQuestion` which rules to move behind `paths:` frontmatter, and what triggers a read of each.
3. **Writers.** Find every agent, command, skill and hook that adds to files in scope. Each must point at the editing directive; add the pointer where missing.
4. **Trim.** For each file:
   - Keep every rule. Reduce each to the rule plus at most one line of why.
   - Remove narration, preambles, arguments, incident stories and duplicates. `domain-lessons.md` keeps each entry's issue number and proof; trim only its wording.
   - Agent files, command files, skills and the user's instruction blocks: keep every instruction exactly; remove only unneeded words.
   - Keep every heading another file cites, or update the citation.
5. **Conflicts.** Compare rules across files, `AGENTS.md` included. Where two disagree, keep one and delete or reword the other. Where the resolution is a product or process decision, ask via `AskUserQuestion` first. Record every intended change of meaning (conflict resolutions, user-directed changes) in `.claude/.trim-allowlist.md`, one line each: file, old rule, new rule.
6. **Parallelize.** One subagent per file group, each editing only its assigned files. Wait for all before verifying.
7. **Verify instructions (semantic diff).** One fresh subagent per file group, dispatched after all edits and conflict resolutions. For each file it:
   1. Writes the original to disk (`git show $BASE:<path>`) and, **before reading the trimmed file**, extracts from that on-disk copy only (never a copy already in context) every rule or instruction into `<scratchpad>/trim/<file>.before.md`: numbered, one atomic directive per line, including conditions, thresholds, commands and exceptions.
   2. Reads the trimmed file and, for each numbered item, writes `<scratchpad>/trim/<file>.map.md`: status (`kept`, `merged`, `weakened`, `changed`, `lost`) and a verbatim quote from the trimmed file (or its new location in another file) that carries it.
   3. Lists instructions present only in the trimmed file.
   4. **Fails the file** if any item is `weakened`, `changed` or `lost` and not in the allowlist, or if a trimmed-only instruction is not in the allowlist, or if any quote does not literally appear.
   5. On failure, restores the meaning in the trimmed file with the fewest words, then re-runs steps 2–4 for that file. Retry at most twice; if it still fails, stop and report the file and items to the user.
   6. Reports per file: item count, counts by status, PASS/FAIL, retries used.
8. **Verify mechanics.**
   - Every cited heading and file path still resolves.
   - If a hook or `bin/` script changed, run its `bin/lib/*.test.sh` and quote the result line.
   - The always-loaded set is within budget; report it and the total including `~/.claude/CLAUDE.md`.
9. **Hand back.** Delete `.claude/.trim-allowlist.md` from the tree, keeping its content for the PR body. Do not commit or push: the user places commits.
10. **Report.** Branch, line counts before and after per file, always-loaded character total before and after, each conflict resolved, the allowlist, and the verification table.
