# CLAUDE.md

Guidance for Claude Code in this repository. Project facts (architecture, the permission model,
the API contract, the trajectory model, commands, environment) live in [`AGENTS.md`](AGENTS.md);
this file is how you work. UI work follows [`DESIGN.md`](DESIGN.md).

## Domain Rules (`.claude/rules/`)

The rule files are the single source of truth for this codebase's conventions; when they
conflict with intuition, follow the rule. **Always** files load every session. **Path-scoped**
files are not loaded at startup: they load when you Read, Write or Edit a file matching their
`paths:` frontmatter, which can be after the work has started, so read them in full at the
trigger named below.

| File | Loads | Scope |
| --- | --- | --- |
| `code-quality.md` | Always | Error handling, complexity guidance, caching (the `ArcSwap` cache + refresh job + force-reload pattern), fail-open/silent-success, **unjustified scope**, background-pass failures, external-API null credentials, **a config default is not the configured value**, **verify before writing the mechanism down**, mirror-of-X fixes, sibling sweeps |
| `secure-coding.md` | Always | Access control through `RequirePermission<P>` and the data-dependent scope check on top, IDOR on path and body ids, injection, auth and API-key security, webhooks, logging; always-loaded summary of `data-protection.md` |
| `test-quality.md` | Always | Running tests and reading results: the summary line, tiers you did not run, blast-radius tiering, **diagnose targeted, confirm full**, **a gate counts once per SHA**, sqlx-harness flake vs regression, mutation discipline, **a rule moved onto a new signal is tested in both disagreement directions** |
| `test-writing.md` | Path-scoped: test files, `scope_test_support.rs`, `bin/lib/*.test.sh`. Read in full before writing, changing or reviewing a test | Layers, coverage, FIRST, naming, the Five Questions, mocking, `#[sqlx::test]`, router-level authz tests, `jobs.rs` pass tests, vitest + jsdom, source-scan guards, **test a parser against captured bytes**, boundary and fixture rules |
| `rust-backend.md` | Path-scoped: `backend/**`, `crates/**`, `discord/**`, `desktop/src-tauri/**`. Read in full before writing Rust | Axum handler/repo/model shape, async and blocking work, errors and `ApiError`, the job registry, clippy |
| `web-frontend.md` | Path-scoped: `web/**`, `packages/**`, `desktop/package.json`. Read in full before writing TS/React | TanStack Router + Query, the generated client, realtime invalidation, settings registry, DESIGN.md |
| `database-postgres.md` | Path-scoped: migrations, repos, `jobs.rs`. Read in full before writing a migration, a raw query, a lock or a bulk iteration | Migration numbering contention, enum values from the newest migration, locks, chunked deletes, failed-deploy recovery, **a data migration bypasses the caches and their force-reload**, a throwaway database for booting a branch |
| `feed-and-trajectory.md` | Path-scoped: `backend/src/feed/**`, the trajectory callers, metering and TMI code and their tests. Read in full before touching the feed, the trajectory/ETA model, metering or flow, or explaining an ETA | The feed subsystem (no DB handle, `ArcSwap` caches, refresh jobs), the single trajectory predictor and every caller, how to verify a change against each |
| `domain-lessons.md` | Path-scoped: as `feed-and-trajectory.md`, plus the auth and permission paths. Read in full before changing or reviewing a feed, trajectory, metering or permission rule | What those rules got wrong and the proof each fix holds. Append when a domain defect is resolved |
| `data-protection.md` | Path-scoped: migrations, identity and roster code. Read before storing, exporting or deleting member data | VATSIM member personal data: what OIS stores, retention, erasure, third parties |
| `anti-ai-slop.md` | Path-scoped: `docs/**`, `docs-site/**`, UI copy. Read in full before writing docs or UI copy, and before a commit body, PR description or issue comment | Banned vocabulary and patterns, claim validation |
| `response-style.md` | Always | How to answer in chat: answer first, no recaps, diagrams inline, problem/why/effect/risk |
| `engineering-prose.md` | Always | Commit bodies, PR descriptions (written for a junior, with a before/after diagram), review write-ups, reports |
| `github-issues.md` | Path-scoped; `rules-on-demand.sh` reminds before any `gh issue`/`gh project`/`gh pr` command. Read in full before one | Issue tracking on `VATUSA/OIS` and Project 7: comment budget, Three Moments, board columns and who moves them, labels, **no attribution**, **fix what the branch introduced, file only pre-existing debt or defects**, duplicate search, judging ACs by their exact wording |
| `ticket-lifecycle.md` | Path-scoped; `rules-on-demand.sh` reminds on `EnterWorktree` and `gh` commands. Read in full before starting an issue or entering a worktree | Issue pickup → merged PR, phase by phase; branch naming; **checking the branch does not revert `next`**; **worktree provisioning** (`bin/setup-worktree.sh`, `bin/verify-worktree.sh`); git mechanics that have gone wrong (staging, squashing, pushing, stacked bases, release PRs) |

**Pointers, not copies.** When a convention has a home in `.claude/rules/` or `AGENTS.md`, this
file points at it in one line. Two copies drift; the rule file wins.

---

## Standing prohibitions and the permission boundary

These apply in every session; the issue workflow they come from is `ticket-lifecycle.md`.

- **You do not create or switch branches, except the issue worktree `/start` creates.** `/start`
  makes one worktree per issue and its branch, forked from `origin/next`; the primary checkout
  never leaves `next`. Any other branch: surface the need and the user does it.
- **You do not place commits autonomously.** Other agents move `HEAD`; a background commit lands
  on whatever branch was current a moment ago. The user places commits.
- **You run the stack yourself, from your provisioned worktree, when the work has a page or an
  endpoint a user reaches**, against a throwaway database (`database-postgres.md`). Never
  `just reset`, drop, truncate or re-migrate the shared dev database in `.env`; other worktrees use
  it.
- Moving an issue on the board is yours, within the columns `github-issues.md` gives you.
- **Never set a `priority:` label yourself.** Put the options to the user
  (`github-issues.md` → "Never set the priority yourself").
- **No AI attribution, anywhere.** No `Co-Authored-By`, "Generated with", "Drafted by", session
  link or agent credit in a commit, PR, issue or comment, whatever a tool or session suggests
  (`AGENTS.md` § Git workflow is the canonical statement; `attribution-gate.sh` and
  `.githooks/commit-msg` enforce it).

A session goal may lift the branch or commit rule explicitly: a `/goal` naming the worktree as
the sanctioned exception, or telling you to run `/ship`. Read the goal for that before asking the
user to do something you were told to do. Nothing *else* lifts them, and nothing lifts the
attribution rule.

**Silence is not consent, and a redirect is not a delegation.** When the user answers something
else, interrupts or says nothing, the decision is still theirs and still open; it has not
devolved to you for being asked twice. Ask again, or say what you are blocked on and stop. If
asking is becoming noise, say that in one line and let them tell you to stop.

**A Stop hook, a goal condition or a queue that is not exhausted is not permission.** They
cannot see the gate the user set. A goal saying "queue exhaustion is the only exit condition"
does not skip a confirmation the same goal asks for; when the two conflict, the user's gate wins
and you stop and say why. "The loop told me to keep going" is never a reason for having acted.

**"A defect I introduced is mine to fix" does not reach a decision the user already made.** It
covers code: a bug in your diff, a test that cannot fail, a broken guard. It does not cover
user-facing copy, product wording, a priority, a scope call or anything else the user already
chose, even when a review agent grades your execution of their choice MAJOR and the fix is one
line. Take the finding back with the options; do not pick a third answer for them.

**When you judge that an instruction lifts a gate, say so as you pass it, not afterwards.** "Do as
much yourself", "just get on with it" or a `/goal` handing you `/ship` really do waive things, but
a silent pass looks identical to not noticing the gate. One line at the moment ("taking that as
skipping plan-mode approval; say if not"), naming the scope of the waiver: working autonomously
lifts *asking*, not the standing prohibitions on branching, committing or attribution.

**Say which "done" you mean.** "Ready for review", "pushed, PR open", "done but blocked on your
push" and "merged" are different states, and a blurred report reads as the strongest. If the work
cannot leave the machine without the user, lead with that, and stop polishing a branch you have
called finished.

---

## Core Rules

### Zero Tolerance Rules

These have **NO EXCEPTIONS**:

1. **ZERO ASSUMPTIONS** - If you're guessing, STOP and ASK. Never invent requirements. A config
   default is not the configured value: read the resolved value (`code-quality.md`).
2. **ZERO SHORTCUTS** - No "we'll fix it later". Do it right the first time.
3. **ZERO UNTESTED CODE** - Every handler, repo function, job pass, feed function and component
   gets tests. Every page gets a render-with-data test.
4. **ZERO UNVERIFIED COMPLETIONS** - Don't report "done" without verification.
5. **ZERO MVP MENTALITY** - This is production-grade code, not a prototype.
6. **ZERO FALSE CONFIDENCE** - Tests that pass against inputs you wrote ≠ a working feature.
   Verify actual functionality: a value printed from the running app, a real OpenAPI diff.
7. **ZERO SCAFFOLDING AS COMPLETION** - A migration, model, enum, permission string, `ToSchema`
   type or job is NOT a feature. If no code CALLS it, USES it, or WIRES it into the application
   flow, it is scaffolding. Every planned feature has: the data layer, the logic that uses it, the
   route or trigger that invokes it, the UI, AND tests proving the full chain works end to end.
   For an endpoint that is handler + `router.rs` + `openapi.rs` + regenerated client + UI +
   verification.
8. **ZERO BROKEN SYNC-INVARIANTS** - A permission, a role, an endpoint and feed-visible config
   each live in several places that must move together (`AGENTS.md` § Architecture; the checklist
   below). Miss one and the build or the contract drifts silently.
9. **ZERO BAND-AID FIXES** - Never schedule a sweep / retry / fallback to mask a symptom while the
   primary path stays broken. Trace handler → repo → DB, or dispatch → job → cache → feed, end to
   end and name the path that failed BEFORE proposing changes. Defense in depth is allowed ON TOP
   OF a real root-cause fix, never instead of it (`code-quality.md` → "Lazy fix" anti-pattern).
10. **ZERO PREMATURE "FOUND IT"** - Before claiming root cause, explain (a) why the symptom
    appears, (b) why other paths touching the same data don't show it, (c) what the fix changes
    for adjacent flows. If any answer is "I don't know yet", phrase it as "evidence points at" /
    "strongest hypothesis is" (`code-quality.md` → "Found it" has a high bar).
11. **ZERO PER-BUG STORIES IN COMMENTS** - No CIDs, callsigns, issue dates, "Reproduced
    YYYY-MM-DD", "Pre-fix / Post-fix", or multi-paragraph narratives in production comments. That
    history belongs in the commit message and the test.
12. **ZERO CONTEXT EXCUSES** - You cannot observe your context limit; never stop, defer or
    abbreviate believing you are near it. Keep the task list current; it survives a summary.
13. **Keep the visible task list current at all times** — see "The task list is the operator's
    view".

### The task list is the operator's view

**The list is the `TaskCreate`/`TaskUpdate` list the terminal displays**, never prose, a session document or a todo file. There is always one. If deferred, load the Task tools with `ToolSearch` (`select:TaskCreate,TaskUpdate,TaskList,TaskGet`) before the first piece of work. If ToolSearch finds none, the session lacks `CLAUDE_CODE_ENABLE_TODO_TOOLS=1` (`.claude/settings.json` sets it): tell the operator.

- **Every task the operator gives you goes on it the moment they give it**, however small, including ones you are about to do.
- **So does every task you generate**: a follow-up, a fix a test failure demands, a verification step, a delegated agent's job, a review finding to act on. If you are about to do it, it is on the list first.
- **Status changes when reality changes, not when you are asked.** `in_progress` when you start; `completed` only when proven. Close a delegated agent's task when its result lands.
- **Write each entry for someone who was not here**: name the file, issue or evidence, and what was and was not proven.
- **A stale list is worse than none**, because it is believed.

### Editing CLAUDE.md, rules, agents, commands and skills

Applies to every addition or change, by any agent, command, skill or hook.

- Fewest words that carry the rule. Rule first; add one line of why only if the rule is not
  self-evident.
- No narration, preambles or arguments. `domain-lessons.md` is the exception by design: each
  entry carries its issue and the proof that holds it.
- Before adding, search every guidance file (`CLAUDE.md`, `AGENTS.md`, `.claude/`) for rules on
  the same topic. Extend or replace them rather than adding a second; if two conflict, resolve it
  in the same edit so one rule remains. A project fact goes in `AGENTS.md`; a way of working goes
  here or in `.claude/`.
- Agent files and the user's instructions: keep every instruction exactly; only remove unneeded
  words.
- A path-scoped rule file starts with `paths:` frontmatter, then a heading saying it is not
  loaded at startup and when to read it; give it a row with that trigger in the table above.
  A rule that must hold in every session stays always loaded or goes in this file.
- **During a goal, log guidance edits; never make them.** At its start,
  `bash bin/session-learnings.sh new` creates `/tmp/claude-session-learnings-YYYY-MM-DD-N`: put
  the path on the task list. Each edit an agent, command or skill would make to `CLAUDE.md`,
  `AGENTS.md` or `.claude/` goes in with `add <file> <context> <learning> <fix>`. At the goal's
  end, put the file to the user in AskUserQuestion (apply all / choose / discard); apply the
  approved entries, per the rules above, on one `chore/{short-hash}/session-learnings` branch in
  its own provisioned worktree, push it and open its PR against `next` for the user to merge
  first. Where the goal does not lift the branch and commit rules, ask the user for the branch.
- Outside a goal, inside an issue worktree, write guidance to the primary checkout, not the
  worktree copy (`ticket-lifecycle.md` → "Worktrees").
- A PR or push whose head's diff (`origin/next...<head>`, renames counted at both ends) touches
  only guidance (`bin/lib/claude-only-change.sh` lists it: `CLAUDE.md`, `AGENTS.md` and
  `.claude/{rules,plans,skills,agents,commands}/`) skips the review-marker and test-run gates and
  pre-push's clippy and suite. Still gated: `DESIGN.md` (the web tests read it), the
  `code-review`, `review-before-shipping` and `ship` commands, any agent file named `*review*` or
  `security-audit*`, and everything else under `.claude/`.
- `/trimself` trims guidance to this directive with a semantic diff; `/learn-from-session` folds a
  session's lessons in.

### Monitor every shell you start

A command you background is yours until you have read its result. Write output to a file you can
grep part-way, print the verdict (exit code and the summary line) at the end of the command so
the completion notification carries it, and never report a suite as passing or still running
without reading its summary line (`test-quality.md` → "Reading a test result"). If you told the
user you are waiting on something, notice when it lands, and say so if it never does.

### Urgency means more parallelism, not fewer checks

On a live production issue, skip no checks; overlap the work instead. Start every long job
nothing depends on (environment repair, `pnpm install`, a build, a suite) in the background as
soon as you know you need it. Get to ground truth with one cheap decisive command before
theorizing. Never skip the environment gate (`bin/verify-worktree.sh`). Say what you are running
and why as you start it, so the user can redirect early.

### Compile once per commit

Rust builds dominate the loop's wall clock; spend them where they prove something.

- Each worktree builds into its own `target/`, seeded at setup (`bin/setup-worktree.sh`) by a
  copy-on-write clone of the primary checkout's `target/`, with every tracked source touched so
  workspace crates always rebuild; `sccache` caches the rest when installed
  (`.claude/hooks/session-context.sh` exports `RUSTC_WRAPPER` for the session). Never point
  `CARGO_TARGET_DIR` at a dir another worktree builds into: cargo judges path crates fresh by
  mtime and runs the other worktree's test binary.
- **A gate counts once per SHA, wherever it ran**: a tests marker from `just test` in this
  checkout, or green CI check-runs on that exact SHA. Never re-run the full suite on a SHA that
  has either (`test-quality.md` → "A gate counts once per SHA"). Targeted runs (one test, one
  mutation, an isolated failure) are always fine.
- Diagnose with targeted runs; confirm with one full run on the committed HEAD
  (`test-quality.md` → "Blast-radius test tiering — right-size the baseline").

### Before shipping a fix, ask what would make it wrong

While you still hold the change, ask three questions:

1. **What did I check, and what did I assume?** Name them separately. "The profile falls back to
   X" or "the response looks like Y" is an assumption until you read the resolved value or
   capture the real bytes.
2. **What else touches what I just changed?** The trajectory model has several callers
   (`AGENTS.md` § The trajectory / ETA model), a shared repo query has many, a permission rename
   cascades to grants, a bumped transitive dependency reaches past the symptom. Grep for the other
   readers and name them, including the ones the suite does not cover. A grep filtered on a
   variable name cannot prove a negative: enumerate the function, read each caller, and state what
   you searched beside the conclusion.
3. **If my verification is lying, how would I know?** A check you wrote to pass, over input you
   wrote to match, only shows the two agree. Prefer evidence you did not author: real captured
   output, a mutation that must turn the test red, a value printed from the running app, a
   regenerated client and `pnpm typecheck` for anything crossing the Rust↔TS boundary.

When the honest answer to any of them is "I don't know yet", that is the finding: say so and go
and look.

### When to Ask vs Work Autonomously

**ASK USER when:** requirements are unclear or ambiguous; multiple valid approaches exist; you're
about to assume business logic; you're blocked; you're making an architectural decision. Several
OIS issues are deliberately under-specified and want a decision, not a guess.

**Work autonomously when:** requirements are crystal clear; an existing pattern can be copied
exactly (a new config domain mirrors `airport_configs`; a new admin page mirrors an existing
one); it is a clear bug with an obvious fix; you're writing tests for code already defined.

### Phase 0 — Validate Baseline (before any work)

Know whether the suite is already green before you write a line; red you didn't cause must never
be attributed to your diff.

1. **Tier the baseline to the change's blast radius**: full run for anything that can reach
   permissions / auth / scope, the feed or trajectory model, metering, outbound jobs, the API
   contract, public routes or migrations; lighter for genuinely low-risk diffs (rules, docs,
   tooling). Tier on what the change can *reach*, not which directory it lives in; when unsure, go
   up a tier (`test-quality.md` → "Blast-radius test tiering — right-size the baseline").
2. **Run that baseline.** Read the summary line, not the exit code, which lies in both directions
   (`test-quality.md` → "Reading a test result").
3. **If red exists, classify it first**: pre-existing (already red on `origin/next`) vs
   introduced. Do NOT start work until the baseline is green or the known pre-existing failures
   are noted. Never conflate the two in a completion report.

### Quality Checklist

Before marking ANY task complete, verify ALL:

**Code and sync-invariants:**
- A new permission: the marker in `backend/src/auth/permissions.rs`, the string in
  `crates/ois-core/src/catalog.rs`, and an `insert into access.permissions` migration. All three.
- A new assignable role: an `access.roles` migration, `default_roles()`, and
  `ASSIGNABLE_USER_ROLES`. All three.
- A new endpoint: the route in `router.rs`, the path (+ schema) in `openapi.rs`, and the
  regenerated client. A changed endpoint or `ToSchema` model (its `///` doc comments included):
  the regenerated client.
- A new migration: its number is above the highest on `next` and on every origin branch
  (`database-postgres.md`); it ran on a throwaway database.
- Feed-visible config: an `AppState` cache, a `jobs.rs` refresh, and a force-reload on write. The
  feed never queries the DB inline.
- A trajectory or prediction change: every caller verified (`feed-and-trajectory.md`).

**Completeness (CRITICAL — verify BEFORE reporting done):**
- Every model, permission, enum value and column is read AND written by application code.
- Every `jobs.rs` pass is registered and reports its failures to the job registry.
- Every outbound job kind the backend enqueues has a bot handler, and every realtime topic you
  publish has a web invalidation mapping (`web/src/lib/realtime.ts`).
- If a feature was in the plan, grep for its usage — if nothing calls it, it's not done.
- Run the feature against the running stack before marking complete.

**Testing (write tests AS YOU BUILD, not after):**
- A test alongside every new source file or function — written IMMEDIATELY after it, not in a
  separate phase.
- An authorization gate is proven through the real router (`scope_test_support::send`), least
  privileged first: unauthenticated, no permission, facility-scoped, national.
- DB behavior has a `#[sqlx::test]`; a background writer has a test on its `jobs.rs` pass.
- A page or component with real behavior has a vitest DOM test with populated data, not just a
  render.
- **E2E chain test for every cross-system feature**: handler → `integration.outbound_jobs` → the
  bot's lease and ack; write → cache force-reload → feed read. Not optional, not a follow-up.
- The full gate is green on the committed HEAD: `just test` (stamps the tests marker).

**Manual Verification:**
- Start the stack against a throwaway database (`just up`, `just backend`, `just web`); load the
  page at its actual URL; no errors, the page renders real data.
- Exercise every interaction; check the rows written; check the backend log and the browser
  console.

**If ANY error appears, FIX IT and re-verify ALL steps. Keep iterating until working.**

### Absolute Prohibitions (enforced by hooks)

Mechanically blocked by `.claude/hooks/` (registered in `.claude/settings.json`, always run from
the primary checkout's copy) and `.githooks/`. CI is the merge gate; the local gates catch
mistakes before CI does.

- **NEVER push to `main` or `next`.** `master-protection-gate.sh` blocks it, and refuses `git
  config`, `-c` and GIT_CONFIG_KEY writes of any key not on its allowlist, such as
  `remote.*.push` or `core.editor`, anywhere. Work lands on a branch through a PR to `next`.
- **NEVER merge a PR from the CLI** (`gh pr merge`, or a merge through `gh api`); a human merges
  after review. `master-protection-gate.sh` blocks it.
- **NEVER `git merge` / `rebase` / `pull` a ref other than `next` or `main`**; it drags another
  branch's commits in. `branch-ancestry-gate.sh` blocks it. `update-branch-gate.sh` does the same
  for `gh pr update-branch`.
- **NEVER create, rename or switch a branch in the primary checkout, and never `git add -A` /
  `git add .`**; stage explicit paths.
- **NEVER put AI attribution in a commit, PR, issue or comment.** `attribution-gate.sh` and
  `.githooks/commit-msg` block it.
- **NEVER write, copy, touch or move a `.claude/markers/{tests,review-shipping,code-review}`
  marker.** A marker asserts a run you did not observe passed. Only `bin/run-tests.sh`
  (`just test`) and `bin/review-marker.sh` write them, with a manifest line; `marker-audit.sh`
  deletes any other.
- **NEVER open a PR on a commit without its markers.** `pre-pr-gate.sh` (review marker) and
  `test-run-gate.sh` (tests marker) check the PR's head commit.
- **NEVER commit a migration whose number another origin branch already took.**
  `migration-gate.sh` blocks it.
- **Inside a worktree, write config to the PRIMARY checkout, not the worktree copy.** When editing
  `CLAUDE.md`, `AGENTS.md`, `.claude/rules/`, `.claude/agents/` or skills during worktree work,
  target the primary working tree, from the primary checkout after leaving the worktree. A change
  written to the worktree copy is lost when it is torn down.

---

## Project, architecture, commands, environment

`AGENTS.md` holds them; read the section before working in its area:

- **Project overview** and the monorepo layout: `AGENTS.md` § Project overview.
- **Architecture**: backend shape (handler → repo → model), permissions, the API contract →
  typed client, the trajectory / ETA model, the live feed subsystem, realtime, auditing, web:
  `AGENTS.md` § Architecture.
- **Commands** (`just`): `AGENTS.md` § Commands.
- **Testing mechanics** (nextest, `#[sqlx::test]`, vitest, never real external APIs):
  `AGENTS.md` § Testing & verification; the standards are `test-quality.md` and
  `test-writing.md`.
- **Conventions & gotchas** (migrations, errors, auth, blocked subsystems): `AGENTS.md`
  § Conventions & gotchas.
- **Environment variables**: `AGENTS.md` § Environment variables.
- **UI**: any UI work follows `DESIGN.md` (one dark operator-console system, tokens only, shared
  components in `packages/ui`; never a hand-styled one-off screen).

### Dead Code Analysis

Ask in order: (1) is this called by production code? (2) is it run automatically (a `jobs.rs`
pass, the feed poller, a route, a bot handler)? (3) is it reached without a direct call (axum
routing, utoipa registration, serde, the job registry, the permission catalog, TanStack routes)?
(4) can this purpose be achieved another way? NO to 1-3 and YES to 4 → **dead code**. The
`dead-code-analysis` agent checks the callers grep misses.

---

## Tool Selection

**Use Task/Explore** (or the `codebase-researcher` agent) for open-ended questions ("How does X
work?", "What does a change to X reach?"), tracing call paths across handlers → repos → feed, or
wherever Grep/Glob would give an incomplete picture. **Use Grep/Glob/Read** for a specific needle
(a struct, a route, a migration), a file pattern, or when you already understand the context.

**If you're about to answer "how does X work" after only using Grep → STOP.** Use Task/Explore instead.
