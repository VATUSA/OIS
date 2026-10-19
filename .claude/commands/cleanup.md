---
description: Remove a finished issue worktree, its throwaway database and its local branch — refuses while anything in it is uncommitted or unpushed.
argument-hint: "[worktree name] (default: the worktree you are in)"
---

You are running a **full local teardown** of a worktree: its throwaway database, the worktree directory, and the local branch. `/ship` does the same thing as its last step; `/cleanup` is for a worktree that never shipped, was abandoned, or was left behind by an earlier session.

This is **destructive and irreversible**. The gates below exist so it never destroys work that is not already safe on `origin`. Follow every step in order.

`bin/teardown-worktree.sh` does the work. For a worktree the current session entered, `ExitWorktree` triggers it through its PreToolUse hook (`.claude/hooks/worktree-teardown.sh`); for any other worktree you run it directly from the primary checkout.

---

## Step 1: Determine the target worktree

Worktrees live under `.claude/worktrees/<NAME>`. `/start` names them after the branch with `/` encoded as `-` (`fix-289-metering-descent-eta`; older worktrees used `+`). Parse the user's message for an explicit name; otherwise use the worktree the session is in.

```bash
pwd
git worktree list
```

- **Explicit name given** → use it.
- **No name given** → the basename of the current `.claude/worktrees/<NAME>` directory. If the cwd is not under `.claude/worktrees/`, **STOP** and ask which worktree to tear down.

The name must match `^[A-Za-z0-9_+-]+$`, the same pattern `bin/setup-worktree.sh` and `bin/teardown-worktree.sh` enforce. The script derives the database name itself; do not hardcode it here.

**Refuse** the primary checkout (the first entry in `git worktree list`, whose `.git` is a directory; it stays on `next` and is never removed), a worktree on `main` or `next`, and one another session is plainly using (you didn't create it and it isn't yours to remove): ask.

A worktree made by hand with `git worktree add` outside `.claude/worktrees/` (an older session's `../ois-wt/<branch>`) gets the same Step 2 and Step 3 checks, then `git worktree remove <WT>` and `git branch -d <BR>` from the primary checkout, never `--force`.

## Step 2: Safety — verify nothing unsaved is about to be destroyed

From the primary checkout, with `<WT>` the absolute worktree path:

```bash
git -C <WT> fetch --prune origin                 # fresh remote refs; a failure goes to Step 3
git -C <WT> status --porcelain                   # a) uncommitted or untracked changes
git -C <WT> rev-parse --abbrev-ref HEAD          # b) the branch ...
git -C <WT> rev-parse HEAD                       #    ... its commit ...
git ls-remote origin refs/heads/<branch>         #    ... and origin's (empty: never pushed)
git -C <WT> log origin/next..HEAD --oneline      # c) commits not on origin/next
```

Inside a worktree session the harness refuses both `git -C` into another checkout and git nested in `$(...)`, so run the same commands plainly, without `-C`, from inside the target.

- **(a) is clean AND either (b) `HEAD` equals origin's SHA or (c) prints nothing** → every commit is on `origin`. Proceed to Step 4; invoking `/cleanup` is the go-ahead for a safe teardown.
- **(a) dirty**, the fetch failed, or commits exist that are on neither `origin/<branch>` nor `origin/next` → Step 3.

Compare against `origin/...` and `git ls-remote`, never the local `next` ref. Ignored files (`.env`, `node_modules`) don't show in (a) and are expected to go.

A ticket-reviewer `chore/<n>/review-<hash>` branch is never pushed under its own name, so an empty `ls-remote` is expected. Judge it against the PR's branch (`gh pr view <pr> --repo VATUSA/OIS --json headRefName`): with (a) clean, `git -C <WT> log origin/<pr-branch>..HEAD --not origin/next --no-merges --oneline` printing nothing means only the review's own `next` merge is local, and nothing is lost. `bin/teardown-worktree.sh` keeps that local branch, since origin has none of its name: once this check passed, delete it with `git branch -D <branch>` from the primary checkout.

## Step 3: Confirmation gate (only when Step 2 found unsaved work)

STOP and tell the user exactly what would be lost:
- "Worktree `<NAME>` has uncommitted changes: `<list>`."
- "Branch `<branch>` has N commit(s) not on origin: `<oneline list>`."
- If `origin/<branch>` is gone after the fetch, first check `gh pr list --repo VATUSA/OIS --head <branch> --state merged`: a merged PR whose branch GitHub deleted is saved work, so say "PR #N merged and its branch was deleted; the work is on `next`" rather than calling them unpushed.

Offer the alternatives (commit and push first, or `ExitWorktree` with `action: "keep"`). Continue only on explicit confirmation. `bin/teardown-worktree.sh` refuses a dirty worktree regardless, so discarding uncommitted changes means the user discarding them first; and it keeps a local branch whose commits are not on origin, which then needs a deliberate `git branch -D`. Never reach for `--force`, `rm -rf` or a stash to get past a check.

## Step 4: Tear down

**The session is inside the target worktree** → call `ExitWorktree` with `action: "remove"`. It refuses when the branch has commits not on the default branch; when Step 2 showed they are on `origin` (or Step 3 got confirmation), re-invoke with `discard_changes: true`. The hook runs the teardown script before the removal and blocks it if the script fails, so a blocked call means nothing was removed: read the message, fix, retry.

**Any other case** (explicit name from the primary checkout, or a worktree from an earlier session, where `ExitWorktree` is a no-op) → from the primary checkout:

```bash
bash bin/teardown-worktree.sh <NAME>
```

Both paths drop the worktree's throwaway database (and read it back), remove the directory, and delete the local branch when origin holds its commits.

**On a Windows host, also remove the branch's WSL clone** (`ticket-lifecycle.md` → "A Windows host"), from the primary checkout's copy:

```bash
wsl.exe -d Ubuntu -- bash -lic 'bash /mnt/c/<primary>/bin/wsl-clone.sh remove <branch>'
```

It refuses a dirty clone or one whose HEAD origin does not hold, as teardown does, then drops the clone's `ois_wsl_` database (read back), deletes `~/ois-wsl/<name>` and runs `fstrim` as root. With no clone it still drops the database. Exit 1 after "removed" means fstrim did not run: run the command it prints from Windows.

## Step 5: Verify and report

From the primary checkout:

```bash
ls -d .claude/worktrees/<NAME> 2>/dev/null && echo "DIR STILL PRESENT" || echo "dir removed"
git branch --list "<branch>"                     # empty, unless kept for unpushed commits
git worktree list                                # target not listed
git worktree prune --dry-run                     # nothing stale left behind
wsl.exe -d Ubuntu -- bash -c 'ls ~/ois-wsl'      # Windows host: the clone is not listed
```

And the worktree's throwaway database is gone. Anything left over: re-run `bash bin/teardown-worktree.sh <NAME>` (idempotent) and check again. A worktree locked by a live session in another terminal is refused before anything is dropped; exit that session first.

Report what was removed and that the session is back in the primary checkout on `next`. State that the **remote branch and any open PR are untouched**: `/cleanup` is local teardown only.

---

## Notes

- After `/ship` there is normally nothing left to clean. `/cleanup <NAME>` is still safe to run then; every step is idempotent.
- **Never point this at the shared dev database in `.env`'s `DATABASE_URL`.** The script only ever drops a name it derived for this worktree; if a derived name matches the shared database, STOP.
- This command touches no issue, board or marker.
