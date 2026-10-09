# Git and worktrees

Loads for every file. The branch model (work lands on `next`, one worktree per issue, never branch
in the primary checkout, PRs target `next`) is in `AGENTS.md` § Git workflow and `CLAUDE.md`
§ Standing working agreements. Branch naming and worktree creation are in
`.claude/commands/start.md`. This file covers the mechanics that have gone wrong around them.

Sources: OIS lessons from #499, #584, #589, #607, #625, #626, #648, #655, #656, and the
October 2026 release PR.

## A fresh worktree can't run the gate yet

`.env` is gitignored and `node_modules` is per worktree, so a new worktree fails in shapes that
look like a broken branch:

- No `.env`: every `#[sqlx::test]` fails with `DATABASE_URL must be set`. Copy `.env` from the
  primary checkout and load it (`set -a; . ./.env; set +a`) before `just ci`.
- No `node_modules`: the JS half dies with `turbo: command not found` after the Rust half passed.
  Run `pnpm install` first.
- Postgres is shared, so the dev database may hold another branch's migrations and refuse to boot
  your backend ("previously applied but is missing"). Boot against a throwaway database on a
  spare port instead of touching the shared one; see `database-postgres.md`.

## Another session may touch your worktree

Several agents work this repo at once, and other sessions have edited and even deleted worktrees
mid-gate. A tracked file you didn't modify, a stray `.bak`, or a compile error in an unrelated
file is the tell. Run `git status --porcelain` before and after every mutation or merge cycle,
and keep backups in your scratchpad, not in the worktree.

## Stage on purpose

- Stage explicit paths. Never `git add -A`: it commits unmerged files with their conflict markers
  (#655 put `<<<<<<<` markers on `next`).
- Before committing a merge or conflict resolution, both of these must come back empty:
  `git diff --name-only --diff-filter=U` and `git grep -n '^<<<<<<< '`.
- Integrity check after every commit: `git status --short` shows nothing you meant to ship.

## Diff against the merge base

`git diff origin/next..HEAD` (two dots) renders everything that merged into `next` since you
forked as a deletion on your side; a two-file docs branch once showed 27 files and 2,258
deletions. Use three dots, `git diff origin/next...HEAD --stat`, or
`git diff "$(git merge-base origin/next HEAD)"..HEAD`. GitHub's own count is
`gh api repos/VATUSA/OIS/pulls/<n> --jq .changed_files`.

## Squash onto a fixed point

`git reset --soft origin/next` or `origin/<branch>` squashes onto whatever that ref points at
**now**, and any `git fetch` in any worktree moves it. The commit then pairs your old tree with
the new base and reverts everyone else's work: 8,646 deletions on #589, about 80 files on #584.

- Squash with `git reset --soft "$(git merge-base HEAD origin/next)"`, the SHA you branched from,
  or `HEAD~N`.
- Before committing, `git diff --cached --stat` must list only your files. A long list means stop.
- Rebase onto the new `next` as a separate step afterwards.

## Never stash in a background command

A background job killed between `git stash` and `git stash pop` strands your work in the stash and
leaves a clean tree that looks like lost edits (#499). The stash stack is also shared by every
worktree and session. To set work aside, commit a WIP checkpoint or copy files to your scratchpad.
Never `git stash` mid-merge either: it drops `MERGE_HEAD`, and the commit comes out with one
parent.

## A push isn't done until the remote says so

Pushes have failed silently: piped through `grep`, or rejected while later `;`-separated steps
moved the card and posted the comment anyway (#648, #656).

- After every push: `git ls-remote origin refs/heads/<branch>` must equal `git rev-parse HEAD`.
  Do this before commenting, moving a card, or removing the worktree.
- Chain the hand-off on the push with `&&`, never `;`. An `echo "done"` at the end of a chain
  proves nothing; verify the resource.

## Shared and stacked branches move under you

- **Before pushing to a branch someone else may hold**, check that it hasn't moved:
  `git merge-base --is-ancestor origin/<branch> HEAD`. Push with
  `--force-with-lease=<branch>:<full-sha>`, never `--force`.
- **A stacked base moves mid-ship** (#625 moved twice while #626 was gating). Right before pushing,
  `git fetch` and `git rev-parse` the base, one ref per call; `git rev-parse a b` fails and quietly
  stops an `&&` chain. Re-rebase only if `git merge-tree --write-tree --name-only <base> <branch>`
  reports a conflict or the base reintroduces what review flagged. Prove the rebase didn't change
  reviewed content with `git range-diff`.

## Merging a batch into `next`

Branches that were green alone break `next` together.

- **Re-probe conflicts right before each merge.** The conflict set goes stale as `next` moves.
- **Run `cargo clippy --workspace --all-targets -- -D warnings` after every merge**, even a
  web-only one. Test modules collide without a textual conflict: a dropped helper, a duplicated
  `use` prologue, a changed type at a call site far from any hunk. Only an `--all-targets` build
  sees them.
- **Don't resolve an append/append conflict by concatenating hunks.** git factors the shared
  trailing lines (`}`, `.await`) out below the marker, so "ours plus theirs" leaves a function
  unclosed (#607). Diff each side against the merge base and rebuild from whole blobs
  (`git show <rev>:<path>`).
- **A red `client-drift` is usually a build failure.** The job dumps the spec by compiling the
  backend (`.github/workflows/ci.yml:139`), so fix the build before regenerating anything.

## Release PRs

Before opening `next` → `main`, check both directions: `git rev-list --count origin/next..origin/main`
above zero means `main` has a commit `next` lacks, such as a hotfix. Promoting without it reverts
the hotfix, silently when the files don't overlap. Merge `origin/main` into `next` first, then
confirm with `git merge-base --is-ancestor origin/main origin/next`.

## `Closes #N` doesn't link on `next`

GitHub honors closing keywords only on the default branch (`main`), so
`gh pr view --json closingIssuesReferences` is always empty for issue PRs. That is not a defect.
Keep writing `Closes #N`; verify the link through the issue timeline:
`gh api repos/VATUSA/OIS/issues/<n>/timeline --jq '[.[] | select(.event=="cross-referenced") | .source.issue.number]'`.
