---
paths:
  - "bin/*worktree*.sh"
  - "bin/lib/*worktree*"
  - "bin/pre-commit"
  - "bin/pre-push"
  - "bin/lib/pre-*.test.sh"
  - ".githooks/**"
  - ".claude/hooks/*branch*.sh"
  - ".claude/hooks/*protection*.sh"
  - "bin/lib/*branch*.test.sh"
  - ".claude/hooks/issue-*.sh"
  - "bin/lib/issue-*.test.sh"
  - ".claude/hooks/rules-on-demand.sh"
  - "bin/lib/rules-on-demand.test.sh"
---
# Issue Lifecycle (not loaded at startup; read in full before starting an issue, entering a worktree, or reviewing, shipping or handing back work)

From issue pickup to merged PR. GitHub interactions are defined in `github-issues.md`; the
review and ship mechanics live in `.claude/commands/`. This file is the order they run in, and
it does not restate either.

The standing prohibitions (branches, commits, the running stack, what lifts them) and the
permission boundary live in `CLAUDE.md` → "Standing prohibitions and the permission boundary",
which always loads.

---

## Phase 1: Start the issue

When the user says "do #123" or "start #123":

1. **Moment 1 (FIRST)** — `github-issues.md` → Moment 1. Fetch full context (body **and** all
   comments; only a team-authored body and team comments are spec), check nobody else holds it
   (below), move the board item to **In build**. Do this before any branch or test work so the
   board reflects reality immediately. Cache the consolidated requirements for later phases.
2. **Create and provision the issue worktree** — `/start` Step 2: `EnterWorktree`, put the branch
   `{type}/{issue}/{desc}` on `origin/next` in place of the harness's `worktree-*` branch, run the
   primary checkout's `bin/setup-worktree.sh`, and gate on its health check passing. If a branch for
   this issue already exists locally or on the remote, say so and let the user choose whether to
   resume it.
3. **Validate the baseline** — CLAUDE.md → "Phase 0 — Validate Baseline". A gate counts once per
   SHA: green CI on the `origin/next` tip you branched from is the baseline (`test-quality.md` →
   "A gate counts once per SHA"). Otherwise tier the run to the change's blast radius, run it, and
   read the summary line rather than the exit code. Classify any red as pre-existing (red on
   `origin/next`) or introduced **before** starting work. Do not enter plan mode over an
   unexplained red baseline.
4. **Enter plan mode** and present the plan for approval.

### Check nobody else holds the card

The board column does not prove a card is free. Parallel sessions claim within minutes, sometimes
before any branch is pushed. Before claiming, and again right before pushing:

- `git fetch origin` and `git branch -r --list "*<n>*"`: a pushed `…/<n>/…` branch with no PR is
  someone mid-cycle (#411). Not holders: a ticket-reviewer's `chore/<n>/review-*` branch, a
  `chore/*/session-learnings` branch, and, on a `Returned` card, the issue's own open PR and its
  head branch, which are the work to recover. On a `To Do` card, the branch and worktree of parked
  work are the work to recover once the user confirms it was parked.
- `git branch --list "*/<n>/*"` and `git worktree list` in the primary checkout: a local branch or
  worktree means someone owns it, even with no remote branch and no card move (#688).
- `gh pr list --repo VATUSA/OIS --state all --search "<n> in:body"`: a combined branch can carry
  another issue's number (#646).
- Re-read **that card's** own status and its latest **team** comment right before every move. A
  listing even a few minutes old has overwritten a `Returned` (#583). If the card is already ahead
  of where you left it, someone else moved it; never move it backwards (#659).

If someone else holds it, read their branch and adopt or extend it, or pick another card. Never
force-push or delete another session's branch without asking.

## Phase 2: Plan approved

1. **Moment 2** — `github-issues.md` → Moment 2. One comment, 600 characters, no attribution of
   any kind. The full plan goes in the PR, not the issue.
2. Exit plan mode and begin Phase 2.5.

## Phase 2.5: Write tests first

Before any implementation code:

1. Dispatch the `tdd-planner` agent with the approved plan and the consolidated issue context.
2. It produces skeletons: Rust `#[ignore = "todo: <requirement>"]` tests with `todo!()` bodies,
   `#[sqlx::test]` skeletons for database behavior, and vitest `it.todo` entries. A page with
   genuine browser behavior also gets its running-stack check written into the plan; it runs last.
3. Run the skeletons targeted. They should compile and report as ignored or todo, not erroring.
4. Implement, making them pass one at a time.
5. Zero `todo` skeletons remaining when the phase closes, and every acceptance criterion maps to
   at least one test.

## Phase 3: Implement

1. Study the existing pattern before writing the new one. Match it exactly.
2. Write the test alongside the source file, not in a later phase.
3. **Regenerate the client after any contract change** (`AGENTS.md` § The API contract → typed
   client), before any typecheck you mean to believe.
4. **Format before the final suite** — `just fmt` (and `pnpm lint` for the web) once
   implementation is done, *then* run the suites. Reformatting after a green run invalidates it,
   and the review commands only check style rather than fixing it, so anything left unformatted
   surfaces as a review finding instead of a non-event.
5. Keep the task list current. New work found mid-task goes on it immediately.

## Phase 4: Pre-completion review

1. **Run `/code-review` yourself** before telling the user the work is ready. It reports
   findings against the diff; on an uncommitted tree it is report-only and stamps nothing.
   Fix what it surfaces. The SHA-keyed markers `/ship` checks come from
   `/review-before-shipping`, run against the committed HEAD.
2. **Do NOT run `/review-before-shipping` or `/ship`.** Those are the user's to trigger,
   unless a session goal has told you to.

   **Take a review agent's first report and act on it.** Every follow-up you send keeps that
   agent alive, and the cost is invisible to you and visible to the user. A truncated report is
   usually enough to act on: the findings are graded and ordered, so the tail is the low grades.
   Ask once, for something specific you cannot proceed without (a gating G1/G2 confirmation, not
   the rest of a list). Never re-dispatch a fresh review because you fixed the findings;
   re-review at ship time against the new SHA.

   **But a dispatched agent that has not reported yet is not a report you may act without.**
   The rule above is about not over-chasing an agent that HAS answered. This is the opposite
   error and the more expensive one: dispatching a review, then reaching a verdict, moving the
   board, posting the comment or placing the commit before it comes back. Once you have asked
   for a second opinion, its absence is not a green light; it is an outstanding question you
   raised yourself.

   **Nothing terminal happens while an agent you dispatched is still running.** Terminal means
   anything that closes work off in someone else's eyes: a verdict, a board transition, an issue
   comment, a commit, a push, a `Closes #` keyword, or telling the user it is done. Do the
   non-blocking work by all means (read the diff, run the gates, draft the report) and hold
   the last step. If an agent is stuck, say so and let the user decide; do not quietly promote
   its silence to agreement.
3. **Verify the diff.** `git diff --stat` for unstaged work and `git diff origin/next...HEAD --stat`
   for the branch. Get unrelated dirty files out of the commit, but never blindly `git checkout --`
   them: the cwd is shared and another session may own that change. Stage only this issue's
   files, by explicit path, or confirm a file is yours and safe to discard before reverting it.
4. **Contamination audit.** `git log origin/next..HEAD --oneline`: every commit must belong to
   this issue. `git diff origin/next...HEAD --stat`: every changed file must sit inside its
   scope. If another issue's work is present, **stop and flag it.** Three dots for the file
   audit, two for the commit audit: `git diff origin/next..HEAD` renders everything that merged
   into `next` since you forked as a deletion on your side, which reads as false contamination (a
   two-file docs branch once showed 27 files and 2,258 deletions). GitHub's own count is
   `gh api repos/VATUSA/OIS/pulls/<n> --jq .changed_files`.

   **Audit against `origin/next`, never the local `next` ref.** A stale local `next` makes
   `next...HEAD` list everything landed upstream since it last moved: other work, in files you
   never touched. It looks exactly like contamination, and the natural reaction (assume the
   branch is poisoned, reset it) destroys good work. Confirm the cause first:
   `git rev-list --count next..origin/next` above zero means your local ref is simply behind.

5. **Check whether the branch would REVERT next.** Contamination is other work arriving in your
   diff; this is the mirror: your diff undoing work that landed while you were building. It is
   invisible to the audit above, because a stale copy of a file looks identical to an authored
   one.

   The exposure is any file the whole repo shares rather than the feature you are on: the hooks
   in `.claude/hooks/`, `bin/`, `.githooks/`, `.claude/rules/`, `CLAUDE.md`, `AGENTS.md`, the
   `justfile`, `.github/workflows/`, `.env.example`, `Cargo.lock`, `pnpm-lock.yaml`. Several agents
   work this repo at once, so these move under you during a single ticket.

   ```bash
   # Files this branch touches that next has ALSO moved since you branched.
   git diff --name-only HEAD...origin/next     # what next moved (three dots: since the fork)
   git diff --name-only origin/next...HEAD     # what this branch touched
   ```

   Two plain commands: inside a worktree session the harness refuses git nested in `$(...)`
   or `<(...)`. Any file in both lists, read `git diff origin/next HEAD -- <file>` and satisfy
   yourself that every line is a change you meant. Bring in `origin/next` (rebase before the
   first push, merge after it) and take next's version of what you did not. A gate loosened to
   unblock your branch is the standing case: it was correct when you wrote it, next has since
   tightened it, and merging quietly puts the loophole back.

   The cheaper answer is usually to drop the file. A tooling fix carried on a feature branch is
   already outside scope (`code-quality.md` → "Unjustified scope"); the ticket only ever needed
   it to get pushed, and the fix belongs on `next` on its own.

## Phase 5: Report to the user

Report all of: what changed against what the issue asked for, the local URL to look at, and
what to check (pages, interactions, edge cases), and which gates ran and which did not. Name the
paths the change reaches: a national-scope and a facility-scope principal are two paths, each
trajectory caller is its own path, and live and historical modes, or web and desktop, differ
where `github-issues.md` → "Moment 3" says they do.

**Curl the URL before handing it over**: `curl -s -o /dev/null -w '%{http_code}\n' <url>` against
the worktree's running backend and Vite server. Anything but 200 means the stack is not serving; fix
it (`bin/verify-worktree.sh`) and re-check. Where a person must look at the change, load that page
on the running stack, confirm the changed element is there, and record the URL and what you saw.
Capture the state that matters: selected, expanded, dialog open, with realistic data volume. A
default page with a thin seed proves nothing.

Move the board item to **Post build** (`bin/board-status.sh <n> "Post build"`): built and
self-reviewed. It stays there through `/ship`; `Code Review` is not yours to set.

**Then wait for sign-off.** `/review-before-shipping` and `/ship` are the user's to trigger.
Never run either off your own bat — not when the tests go green, not when `/code-review` comes
back clean, not because the work feels finished. Say the work is ready, which "done" you mean
("ready for review", "pushed, PR open", "merged" are different states), and what you ran.

## Phase 6: Ship (after sign-off)

Run `/ship`. It owns the order: the gate (`just test`, once per SHA), commit,
`/review-before-shipping`, push, PR against `next`, then **Moment 3** (`github-issues.md` →
Moment 3): verification comment (1,200 characters, route paths only, no attribution), board item to
**Post build**, follow-ups only where all four scope tests pass. Do not reimplement any of it here.

Things `/ship` does not own:

- **Merging.** A human merges the PR through the GitHub UI after review; the CLI path is
  blocked by `master-protection-gate.sh`, and that is correct, even with the owner's one-time
  permission. `main` and `next` are both protected: never pushed to, never merged by an agent.
- **The board after `Post build`.** Under the ticket loop `ticket-reviewer` claims the card into
  `In Test`, then moves it to `Code Review` or `Returned` on the operator's relayed decision
  (`github-issues.md` → "Board reference"). `Shippable` and `Done` are a human's.
- **A merge to `next` closes nothing.** GitHub honors `Closes #N` only on the default branch
  (`main`), so `gh pr view --json closingIssuesReferences` is always empty for issue PRs. Keep
  writing `Closes #N`; verify the link through the issue timeline:
  `gh api repos/VATUSA/OIS/issues/<n>/timeline --jq '[.[] | select(.event=="cross-referenced") | .source.issue.number]'`.

If the work comes back as `Returned` (a failed check, a review finding, a walk showing a defect),
treat that as a fresh Phase 3, not a patch: read why, fix, and re-run Phase 4 before handing back
again. A reviewer may return your card on a stale read: #531 was moved back 81 seconds after its
PR existed. Settle it with timestamps (the comment's `created_at` against the PR's `createdAt`),
restore the column, and correct any substantive claim the stale comment made.

---

## Branch naming

`{type}/{issue-or-hash}/{2-4-word-description}`. Types: `feat`, `fix`, `chore`. Max 50
characters. The regex is `^(feat|fix|chore)/[a-z0-9]+/[a-z0-9]+(-[a-z0-9]+){1,3}$`, so a one-word
or five-word description fails. Enforced with it by `branch-naming-gate.sh`, on the local branch, and
`bin/pre-push`, on the remote ref being pushed. Keep the two regexes byte-identical if you
change either: two gates that disagree block a valid name while the rule file says it is fine.

- `feat/47/save-event-replays` (26 chars)
- `fix/312/favorites-qa-rework` (27 chars)
- `chore/1617e87/aviodeck-workflow-sync` (36 chars: no issue, so a short hash)

**The middle segment is required.** An issue number where there is an issue, a short hash where
there is not, so every branch says on sight what it belongs to. A two-segment name
(`chore/update-deps`) is rejected; rename with `git branch -m`.

**Never push `main` or `next`.** `master-protection-gate.sh` blocks both. If you are on either,
stop and ask for a branch rather than creating one.

## Worktrees

**Use the `EnterWorktree` / `ExitWorktree` tools, not raw `git worktree add` / `remove`.** The
harness tracks worktree entry and exit; going around it with git leaves that state wrong and
breaks teardown.

**Then provision it — a fresh worktree is an empty checkout.** `bin/setup-worktree.sh` does the
whole job and is idempotent, so re-running it repairs drift rather than starting over:

```bash
bin/setup-worktree.sh <worktree-dir-name>   # .env, node_modules, a seeded target/, sccache
bin/verify-worktree.sh                      # run from inside the worktree
bin/teardown-worktree.sh <name>             # drops its throwaway database, removes the dir
```

**Run `bin/verify-worktree.sh` before you believe any gate result from inside a worktree.** An
unconfigured worktree does not fail loudly; it fails in the shape of a branch regression. No
`.env` makes every `#[sqlx::test]` fail with `DATABASE_URL must be set`, and no `node_modules`
kills the JS half with `turbo: command not found` after the Rust half passed. Both read as
something your diff did.

**Each worktree builds into its own `target/`**, seeded at setup by a copy-on-write clone of the
primary checkout's `target/`, with every tracked source touched right after so workspace crates
always rebuild while dependencies are reused. `RUSTC_WRAPPER=sccache` caches the rest when
`sccache` is on `PATH`; the SessionStart hook exports it, and `bin/run-tests.sh`, `bin/pre-push`,
`bin/mutate.sh` and the `justfile` set it. Never export a `CARGO_TARGET_DIR` that another worktree
builds into: cargo judges path crates fresh by mtime, so a worktree ran another's test binary and
passed on code it did not have. `bin/verify-worktree.sh` fails on one.

**And one thing it does NOT check.** A green health check means the worktree is provisioned on the
points above, nothing more:

- **The shared dev database may hold another branch's migrations** and refuse to boot your backend
  ("previously applied but is missing"). Boot against a throwaway database on a spare port
  (`database-postgres.md`), never by re-migrating or resetting the shared one.

- **Creating the issue worktree is yours, through `/start`.** The branch prohibition protects
  the *primary checkout*, which stays on `next`. `/start` creates the worktree and `/ship` (or
  `/cleanup`) removes it with its throwaway database and local branch; asking the user to do
  either is a round-trip that buys nothing.
- **`EnterWorktree` names the branch `worktree-{name}`, and `ExitWorktree` deletes only that
  name.** `/start` replaces it with the issue branch, so the issue branch is deleted by
  `bin/teardown-worktree.sh` (only when origin holds its commits), not by the harness.
- **The session cwd is SHARED and pins to whoever entered a worktree last.** Every agent in a
  session shares one working directory. With several worktrees open, a bare `cd`, a relative
  path or an unqualified `git` command lands in whichever entered most recently. **Use absolute
  paths.** Afterwards check `git status` in both the worktree and the primary checkout: a stray
  write to the primary checkout on `next` is the failure this prevents, and it is silent.
- **Another session may touch your worktree.** Other sessions have edited and even deleted
  worktrees mid-gate. A tracked file you did not modify, a stray `.bak` or a compile error in an
  unrelated file is the tell. Run `git status --porcelain` before and after every mutation or merge
  cycle, and keep backups in your scratchpad, not in the worktree.
- **Inside a worktree session the harness confines git and edits to that worktree.** It refuses
  `git -C` into another checkout, git nested in `$(...)`, `<(...)` or a heredoc fed to bash,
  and Edit/Write to a primary-checkout path. Run git as plain commands with values written in
  literally, and put multi-step git logic in a `bin/` script invoked as
  `bash <absolute-or-relative path>` (`bin/review-scan.sh`, `bin/review-marker.sh`), which the
  harness does allow.
- **A worktree runs its OWN copy of `.claude/rules/` and `.claude/commands/`, but the primary
  checkout's copy of the hooks** (`.claude/settings.json` resolves them via `--git-common-dir`). So
  a branch cut before a rule landed keeps following the old rule indefinitely and nothing warns
  you. When a rule change governs what gets posted, pushed or transitioned, merging it to `next`
  does not deliver it: bring `origin/next` into the open worktrees (rebase if unpushed, merge if
  pushed), or accept they will not comply. The same holds for `bin/`: a branch whose
  `bin/review-marker.sh` predates the marker manifest has every stamp deleted by the primary's
  marker audit, so merge `origin/next` before stamping. Run the stamp alone on its line; chained
  after other commands, the audit reverts it too (#872).
- **Write config to the PRIMARY checkout, not the worktree copy.** A change to `CLAUDE.md`,
  `.claude/rules/`, `.claude/agents/` or a skill written to the worktree copy dies with the
  worktree. The harness will not let a worktree session edit the primary checkout, so note the
  change, and make it from there once the session has left the worktree (or from a separate
  session there). During a goal, log it with `bin/session-learnings.sh` (`CLAUDE.md` → "Editing
  CLAUDE.md, rules, agents, commands and skills").
- **Rebase onto `origin/next` only before the first push; once a branch is pushed, merge
  `origin/next` into it.** A rebase of a pushed branch needs a force-push and rewrites what a
  reviewer already read.
- **Only ever rebase or merge `next`** (and `origin/main` into `next`'s release branch, below).
  `branch-ancestry-gate.sh` blocks merging or rebasing any other ref. If you need another branch's
  changes, they go to `next` first.

## Stage on purpose

- Stage explicit paths. Never `git add -A`, `git add .` or `git add --all`: they commit unmerged
  files with their conflict markers (#655 put `<<<<<<<` markers on `next`), and the gates refuse
  them.
- Before committing a merge or conflict resolution, both of these must come back empty:
  `git diff --name-only --diff-filter=U` and `git grep -n '^<<<<<<< '`.

## Squash onto a fixed point

`git reset --soft origin/next` or `origin/<branch>` squashes onto whatever that ref points at
**now**, and any `git fetch` in any worktree moves it. The commit then pairs your old tree with
the new base and reverts everyone else's work: 8,646 deletions on #589, about 80 files on #584.

- Squash with `git reset --soft <sha>`, where `<sha>` is the merge base you read first
  (`git merge-base HEAD origin/next`, as its own command) or `HEAD~N`.
- Before committing, `git diff --cached --stat` must list only your files. A long list means stop.
- Rebase onto the new `next` as a separate step afterwards.

## Never stash in a background command

A background job killed between `git stash` and `git stash pop` strands your work in the stash and
leaves a clean tree that looks like lost edits (#499). The stash stack is also shared by every
worktree and session. To set work aside, commit a WIP checkpoint or copy files to your scratchpad.
Never `git stash` mid-merge either: it drops `MERGE_HEAD`, and the commit comes out with one
parent.

## Shared and stacked branches move under you

- **Before pushing to a branch someone else may hold**, check that it has not moved:
  `git merge-base --is-ancestor origin/<branch> HEAD`. Push with
  `--force-with-lease=<branch>:<full-sha>`, never `--force`.
- **A stacked base moves mid-ship** (#625 moved twice while #626 was gating). Right before pushing,
  `git fetch` and `git rev-parse` the base, one ref per call; `git rev-parse a b` fails and quietly
  stops an `&&` chain. Re-rebase only if `git merge-tree --write-tree --name-only <base> <branch>`
  reports a conflict or the base reintroduces what review flagged. Prove the rebase did not change
  reviewed content with `git range-diff`.

## Merging a batch into `next`

Branches that were green alone break `next` together. Merging into `next` is a human's; this is
for the integration branch a human asks you to prepare.

- **Re-probe conflicts right before each merge.** The conflict set goes stale as `next` moves.
- **Run `cargo clippy --workspace --all-targets -- -D warnings` after every merge**, even a
  web-only one. Test modules collide without a textual conflict: a dropped helper, a duplicated
  `use` prologue, a changed type at a call site far from any hunk. Only an `--all-targets` build
  sees them.
- **Do not resolve an append/append conflict by concatenating hunks.** git factors the shared
  trailing lines (`}`, `.await`) out below the marker, so "ours plus theirs" leaves a function
  unclosed (#607). Diff each side against the merge base and rebuild from whole blobs
  (`git show <rev>:<path>`).
- **A red `client-drift` is usually a build failure.** The CI job dumps the spec by compiling the
  backend (`cargo test -p ois-backend --lib openapi::dump_openapi_json`), so fix the build before
  regenerating anything.

## Release PRs

A human opens the `next` → `main` release PR. Before preparing one, check both directions:
`git rev-list --count origin/next..origin/main` above zero means `main` has a commit `next` lacks,
such as a hotfix. Promoting without it reverts the hotfix, silently when the files do not overlap.
Merge `origin/main` into `next`'s release branch first, then confirm with
`git merge-base --is-ancestor origin/main origin/next` once it lands.

## Pitfalls

- **`git checkout` destroys uncommitted work, and some of this tree is untracked.** Check
  `git status --short` for `??` entries before any checkout that could clobber them.
- **A rebase dirties unrelated files.** Run `git diff --stat` afterwards and `git checkout --`
  only what you have confirmed the rebase touched; another session may own any other change.
- **Integrating `next` surfaces next's newer enforcement tests against your additions** (the
  `migration_versions_are_unique` test, the ratchet tests in `backend/src/handlers/*_tests.rs`).
  Those are branch-introduced conformance failures to fix, not pre-existing.
- **Check a baseline failure against `next` from the primary checkout**, which already sits on
  `next`, or from CI on `origin/next`'s SHA: never `git checkout next` inside the worktree.
- **After any commit, check it holds your changes**: `git diff origin/next...HEAD --name-only`
  and `git status --short`. Every intended file is in the diff, nothing intended is still
  modified, nothing you did not mean to ship is staged.
- **A push is not done until the remote says so.** Pushes have failed silently, piped through
  `grep`, or rejected while later `;`-separated steps moved the card and posted the comment anyway
  (#648, #656). After every push, `git ls-remote origin refs/heads/<branch>` must equal
  `git rev-parse HEAD`, before you comment, move a card or remove the worktree. Chain the hand-off on
  the push with `&&`, never `;`. The pre-push hook runs clippy and the suite unless `just test`
  stamped the pushed commit in that checkout; a guidance-only push (`bin/lib/claude-only-change.sh`)
  skips both. It refuses a push of any commit but HEAD, or of two branches at once.
- **The Discord bot does not run for a worktree unless you start it** (`just bot`, which needs
  `DISCORD_BOT_TOKEN`), so an enqueued job sits in `integration.outbound_jobs` while you walk an
  AC. Read the row, or run the bot for the walk and stop it after.
- **Tests failing for environment reasons after three debug attempts: stop and ask the user.**
- **A rebase that moved `pnpm-lock.yaml` leaves `node_modules` stale.** `pnpm audit` and the
  typecheck read what is *installed*, not what is committed. Run `pnpm install` before diagnosing
  an audit or type failure.
- **Never remove tooling to bypass a check.** If clippy, rustfmt or a hook blocks a commit, fix the
  error, or use `--no-verify` for a *confirmed pre-existing* failure, verified against
  `origin/next` and said so in the PR. Never uninstall the tool, delete the config, or add an
  `#[allow(...)]` without a stated reason to hide an error your change introduced.
