---
description: Remove a finished issue worktree and its local branch — refuses while anything in it is uncommitted or unpushed.
argument-hint: "[worktree path or branch] (default: the worktree you are in)"
---

You are removing an issue worktree and its local branch. Use it after `/ship` once the worktree is no
longer needed, or for one that was abandoned or left behind by an earlier session.

This is **destructive**. The checks below exist so it never deletes work that isn't already on
`origin`: if any check fails, **stop and report it**. Never reach for `--force`, `git branch -D`
before the checks pass, `rm -rf`, or a stash to get past one. The remote branch and any PR are never
touched; this is local cleanup only.

## Step 1 — Resolve the target

```bash
git worktree list --porcelain
```

- `$ARGUMENTS` is a path → the worktree at that path.
- `$ARGUMENTS` is a branch → the worktree whose `branch refs/heads/<branch>` line matches.
- Empty → the worktree you are in (`git rev-parse --show-toplevel`).

Record its absolute path `<WT>` and branch `<BR>`. **Refuse** when:

- `<WT>` is the primary checkout: the first entry in the list, the one whose `.git` is a directory.
  The primary checkout stays on `next` and is never removed.
- `<BR>` is `main` or `next`: those are never removed here.
- The worktree is detached (a `detached` line, no `branch`), as a reproduction worktree on
  `origin/main` is: there is no `<BR>`, so Step 3 skips the `ls-remote` line and Step 4 skips
  `git branch -d`.
- Another session is plainly using it (you didn't create it and it isn't yours to remove): ask.

## Step 2 — Nothing uncommitted

```bash
git -C <WT> status --porcelain
```

It must print nothing. Any line, tracked or untracked, is work that exists only in that directory:
refuse, and list the files. Ignored files (`.env`, `node_modules`, `target/`) don't show and are
expected to go.

## Step 3 — Nothing unpushed (ls-remote, not the local tracking ref)

Ask the remote itself; `origin/<BR>` in the local repo is only as fresh as the last fetch:

```bash
git -C <WT> rev-parse HEAD
git ls-remote origin refs/heads/<BR>
```

- The remote SHA **equals** the local HEAD → every commit is on `origin`. Go to Step 4.
- Otherwise (the branch was never pushed, was deleted after merge, differs, or the worktree is
  detached) the commit is safe only if some branch on `origin` already contains it. Fetch first, so
  the remote-tracking refs are the remote's state now:
  ```bash
  git fetch origin --prune
  git -C <WT> branch -r --contains HEAD
  ```
  It lists a remote branch (`origin/next` after a merge, `origin/<pr-branch>` for a review worktree
  checked out at a PR's head, `origin/main` for a reproduction worktree) → safe; name that branch in
  the report. It lists nothing → **refuse**: those commits exist only here. Report HEAD, the remote
  SHA if any, and the commits at risk (`git -C <WT> log --oneline HEAD --not --remotes=origin`).

On a refusal, offer the ways forward (push first, or keep the worktree) and stop. Discarding work is
the user's decision, made explicitly, never this command's.

## Step 4 — Remove

From the primary checkout or another worktree, never from inside `<WT>`:

```bash
git worktree remove <WT>
git branch -d <BR>
```

`git worktree remove` without `--force` refuses a worktree with modified or untracked files, a second
line of defense after Step 2. `git branch -d` refuses a branch that isn't merged into its upstream or
the current HEAD. If it refuses only that way, after Step 3 proved its HEAD is on `origin`,
`git branch -D <BR>` is then safe; say that you used it and which remote branch holds the commit.

## Step 5 — Verify and report

```bash
git worktree list            # <WT> is not listed
git branch --list "<BR>"     # prints nothing
ls -d <WT> 2>/dev/null && echo "DIR STILL PRESENT" || echo "dir removed"
git worktree prune --dry-run # nothing stale left behind
```

Report what was removed, the SHA that is safe on `origin` (or in `next`), and that the remote branch
and any open PR are untouched. This command touches no issue, board card or review marker.
