# Creating issues & tickets

How we file and track work for OIS on GitHub. Modeled on the AvioDeck conventions, adapted to
OIS's domains and workflow. This is the standard for both humans and agents.

- **Repo:** [`VATUSA/OIS`](https://github.com/VATUSA/OIS) (public: anyone can comment, so only team
  comments are spec; see `/start`) — issues live here.
- **Board:** [VATUSA · Project 7](https://github.com/orgs/VATUSA/projects/7/views/1) — every issue is
  added to it and moves through its Status pipeline.

An issue is a small, self-contained specification. A good one lets someone who wasn't in the
conversation pick it up cold: it states the observable problem with evidence, explains the
mechanism, says what "done" looks like, and is labeled so the board sorts it correctly.

---

## The board

**Status** (the pipeline — one per issue, in flow order):

| Status | Meaning |
| --- | --- |
| **Blocked** | Can't proceed — waiting on a dependency or a decision. |
| **Triaging** | Being classified, scoped, sized, and prioritized. New issues start here. |
| **To Do** | Triaged and cleared to start. |
| **Returned** | Kicked back for rework (from review or test). |
| **In build** | Actively being implemented. |
| **Post build** | Build done — pre-test wrap-up (regenerate the client, apply migrations, self-check, `just ci`). |
| **Testing Queue** | PR open, awaiting test. |
| **In Test** | Under test. |
| **Code Review** | Tested, PR in review. |
| **Shippable** | Approved and ready to ship. |
| **Done** | Merged / deployed. |

**Size** (rough effort, set at triage): `XS · S · M · L · XL`. XS ≈ a few lines; L/XL should usually
be split into sub-issues (the board supports **Parent issue** / **Sub-issues progress**).

Set **Size** during **Triaging**; link the PR (**Linked pull requests**) when one opens so the board
tracks it automatically.

---

## Labels

Every issue carries **one `type:`, one `area:`, one `priority:`**, plus any modifiers.

**`type:`** — what kind of work it is
- `type:bug` — something behaves wrong
- `type:feature` — new capability or enhancement
- `type:chore` — tooling, deps, config, migrations-only, housekeeping
- `type:documentation` — docs/specs

**`area:`** — the OIS domain it lives in (mirrors the backend schemas / product areas)
- `area:access` — identity, permissions, roles, API keys, audit
- `area:events` — event operations: coordination, staffing, sign-up, debrief
- `area:tmu` — NTML / advisories / TMIs / ground stops / rate programs / GDPs
- `area:flow` — FCAs, metering, routes, runway balancer, IDST, the trajectory/ETA model
- `area:ace` — ACE support requests
- `area:stats` — statistics, replay, delays, dashboards
- `area:integrations` — VATSIM, VATUSA, Discord, email
- `area:web` — frontend that isn't domain-specific (shell, nav, settings, shared UI)
- `area:infra` — build, CI, deploy, migrations, tooling

**`priority:`** — urgency
- `priority:critical` — production broken or data at risk; drop everything
- `priority:high` — major defect or blocking work; next in the queue
- `priority:medium` — scheduled, not urgent
- `priority:low` — minor; picked up opportunistically
- `priority:trivial` — nitpick; fix if you're already in the file

**Modifiers** (add as needed)
- `technical-debt` — a pre-existing defect filed as its own ticket rather than folded into unrelated
  work (see "Scope" below)
- `blocked-by-code` — parked by triage: needs code that only exists on an unmerged branch; nobody is
  building it yet
- `dependencies` — dependency updates
- `good first issue` / `help wanted` — as usual

OIS has no subscriber tiers, so there is no `tier:` label.

---

## Titles

A title is a one-line statement of the problem, specific enough to tell apart from its neighbors —
ideally **symptom → consequence**. Name the real thing, not the fix.

- ✅ `FCA metering uses cruise speed during descent, so arrival ETAs run early`
- ✅ `Adding a permission without the catalog entry 400s the access editor`
- ❌ `Fix metering` · ❌ `ETA bug` · ❌ `Improve permissions`

---

## Body structure

Use these sections (drop ones that don't apply — e.g. a pure feature has no "Why the neighboring
path is fine"). Ground every claim in evidence: real file:line references, real captured output, not
"it should."

```markdown
## What happens
The observable problem. Point at the exact code — `path/to/file.rs:123` — and show it. For a bug,
prove it with real output/behavior, not a description of it.

## Why
The mechanism / root cause. Which line, why the guard doesn't fire, what the wrong value is.

## Why the neighboring path is fine   ← for bugs, when relevant
Why the adjacent code that touches the same data does NOT show the problem. (Forces you past a
premature "found it" and documents the blast radius.)

## What should happen
The fix direction and the trade-off it makes — not a full patch, but enough that the picker isn't
re-deciding the approach.

## Acceptance
- [ ] Concrete, checkable outcomes.
- [ ] Include a test whose failure would catch a regression — ideally one that turns red if the fix
      is reverted.

<!-- footer -->
Blast radius: <does this touch the trajectory/ETA model · permissions (3-in-sync) · the API
contract (client regen)? name it, or "none">
Pre-existing; found while <what you were doing>.   ← provenance, when it's incidental
Relates to #N / Duplicate of #N.                    ← after a duplicate search
```

Two OIS-specific habits in that footer:

- **Blast radius.** OIS has a few changes that reach further than they look — the single
  trajectory/ETA model in `feed/trajectory.rs` (three callers), the permission/role
  "three-places-in-sync" invariants, and the OpenAPI→client contract (needs a regen). If the issue
  touches one, say so; if it touches none, say "none." (This is OIS's analog of AvioDeck's
  "Data path" line.)
- **Provenance.** If you noticed the problem while doing something else, say so ("Pre-existing;
  found while QA-ing #42") and label it `technical-debt`.

### Example (template — not a real bug)

> **Title:** `Access editor 400s when a permission string lacks a catalog entry`
>
> **## What happens** — Saving a grant for `flow.aircraft_profiles.update` returns
> `400 bad_request` from `POST /api/v1/admin/users/{cid}/access`. The handler validates every
> submitted permission against the catalog (`repos/access.rs::fetch_access_catalog_names`), and the
> string isn't in it.
>
> **## Why** — The permission has a marker in `auth/permissions.rs` and a migration row in
> `access.permissions`, but was never added to `default_roles()`/the catalog list in
> `crates/ois-core/src/catalog.rs`, so the editor's catalog validation rejects it.
>
> **## What should happen** — Add the string to `catalog.rs`. More broadly, a lint/test should fail
> when a `permission!` marker or an `access.permissions` row has no catalog entry, so the three
> stay in sync.
>
> **## Acceptance**
> - [ ] The grant saves.
> - [ ] A test enumerates markers + catalog and fails if they diverge.
>
> Blast radius: permissions (three-in-sync). Pre-existing.

---

## Scope — when a thing you noticed becomes its own ticket

While working, you will find adjacent problems. Do not silently fold them into an unrelated change.
File a separate `technical-debt` issue when **any** of these is true:

1. It has a **different root cause** than the work in hand.
2. Fixing it now **materially widens the diff or the risk** of the current change.
3. It is **independently shippable and testable** on its own.
4. Folding it in would **hide it from review** (the reviewer came for change A and gets B too).

If none are true, fix it inline and mention it in the PR. When in doubt, file it — a small tracked
ticket beats a surprise in a diff.

**Before filing, search for a duplicate**, closed issues included (see
[Searching for duplicates](#searching-for-duplicates)). Link related issues (`Relates to #N`) and
mark true duplicates `status`/close with a pointer rather than filing again.

---

## Lifecycle & agent etiquette

- **New issues start in Triaging** with `type` + `area` + `priority`. A human classifies them, sets
  **Size**, and moves them to **To Do** — that's the signal an issue is cleared to start. **Blocked**
  is for anything waiting on a dependency/decision; **Returned** is where review or test kicks work
  back.
- **Agents do not self-assign, close issues, or merge PRs, and don't move an issue to Shippable or
  Done** — a human owns review, ship, and close. An agent may move **To Do → In build** when it
  genuinely starts, run the **Post build** wrap-up (client regen, migrations, `just ci`), open the
  PR, and move the card to **Testing Queue**. Work lands on **`next`**, the integration branch, per
  the project's no-branch rule (see `AGENTS.md` § Git workflow); `main` is promoted from `next`
  separately.
- **The columns say where work is. They do not gate the merge.** What gates a merge is green CI plus
  the review loop — nothing checks a card's column before a PR can land, and nothing is going to:
  enforcing it would need a project-scoped secret (Actions' default `GITHUB_TOKEN` cannot read
  Projects v2) plus a Projects query on every PR event, against an API budget that board polling
  already exhausts.

  So a PR that merged while its card still read **Testing Queue** is a **bookkeeping error to
  correct, not a policy breach**. Correcting it means **saying so on the issue** — an agent still
  does not move a card to **Shippable** or **Done**, even when the PR is demonstrably merged, because
  that transition is a human's (above). Leaving a merged issue sitting in **Returned** is the one
  outcome to avoid: it reads as "needs rework" and invites a second agent to redo work that is
  already on `next`.
- **Comment sparingly** — an issue is a spec, not a chat log. Comment only at the moments and
  within the budget in [Comments](#comments-the-three-moments-and-the-budget) below.
- **No AI attribution:** an agent-drafted issue, comment, commit or PR carries no AI attribution of
  any kind: no `Drafted by` or `Generated with` footer, no `Co-Authored-By` trailer, no session link.
- **Other repos are read-only.** Reference AvioDeck (or any other repo) for patterns, but never
  create, edit, comment on, or label issues outside `VATUSA/OIS`.

---

## Referring to an issue

**Every mention of an issue carries three parts: `#123 [short summary] (Status)`.**

```
#742 [no .claude/rules for agent standards] (In build)
#569 [duplicate migration numbers half-migrate the database] (Done)
```

A bare `#742` makes the reader open a tab to learn what it is, and a list of them is unreadable.
The summary is yours to shorten: enough of the title to identify it. The status is the board
column verbatim (`Triaging`, `To Do`, `In build`, `Post build`, `Testing Queue`, `In Test`,
`Code Review`, `Returned`, `Shippable`, `Blocked`, `Done`), not the GitHub open/closed state.

The status is the part people carry from memory and get wrong, because cards move between reading
the board and writing about it. Read it fresh from the issue's own project item:

```bash
gh issue view 742 --repo VATUSA/OIS --json projectItems \
  --jq '.projectItems[] | select(.title == "OIS Kanban") | .status.name'
```

This applies everywhere an issue is named: chat, reports, PR descriptions, commit bodies, and
issue comments.

---

## Comments: the three Moments and the budget

An issue is read by whoever picks the work up next year. It is not a development log.

### The three Moments

Every issue has three touchpoints.

1. **Moment 1, start work.** Read the body and every comment, check nobody else holds it, and move
   the card to **In build** (`/start`). No comment.
2. **Moment 2, plan approved.** One comment: what you are building and any decision that changes
   what the issue asked for. Not the file list, the test plan, or the sequencing; the full plan
   goes in the PR.
3. **Moment 3, work complete.** After `/ship` has pushed and opened the PR, move the card to
   **Testing Queue** and post one comment. Draft to this template and check every line against the diff:

   ```
   Done: <one sentence on what changed>. PR #<n>.

   How to check:
   1. <a step naming a real route path, file, or control from the diff>
   2. <step>

   Blast radius: <trajectory/ETA model · permissions/roles three-in-sync · API contract · none>
   Deploy: <migration NNNN applies on backend start · client regenerated · new env var · nothing>
   ```

   Use real file paths and route paths (`/admin/access`), never a host. Every described behavior
   traces to code in the diff; if you can't point at it, drop the line. If the change is entirely
   `docs/`, tooling, or test-only, say so and why it needs no runtime verification. Don't fire
   Moment 3 while more work is coming.

### The budget

At most one comment per purpose per round of work:

| Purpose | When | Budget |
| --- | --- | --- |
| Plan (Moment 2) | the plan is approved | 600 characters |
| Spec correction | the issue states something your diff proves false | 400 characters |
| Verification notes (Moment 3) | `/ship` pushed and opened the PR | 1,200 characters |
| Failure response | verification failed and you fixed it | 1,200 characters |

The budget counts the whole comment. **Count characters, not bytes, and count
before you post.** `wc -c` counts bytes, and `—` or `→` is three of them. Make the count a gate
that stops the post, not a message printed above it:

```bash
LIMIT=1200   # the row's budget: 600 for a plan, 400 for a spec correction
python3 -c 'import sys; n = len(open("body.md").read().rstrip()); print(n); sys.exit(n > int(sys.argv[1]))' "$LIMIT" \
  && ! grep -qiE 'Drafted by|Generated with|Co-Authored' body.md \
  && gh issue comment <n> --repo VATUSA/OIS --body-file body.md
```

Draft to about 1,000 characters to leave room. When you are over, don't compress the prose; move
material into the PR. What survives a trim, in order: the check steps with real paths, the blast
radius, the deploy note, and the one finding a reader can't get from the diff.

A spec correction that needs more than 400 characters is a scope change: raise it with the user
instead of writing an essay on the issue.

### What goes in the PR instead

The issue answers "what changed and how do I check it". The PR answers "how was it built". On the
PR side: implementation reasoning, alternatives considered, why a review suggestion was not taken,
anything about tests (suite results, coverage, mutation proofs), anything about getting the code
onto `next` (rebases, conflicts, stacking), notes addressed to a reviewer, and incidental tidy-ups.
Link the PR once; GitHub cross-links it both ways.

### Comments never to post

There is no status-update comment. If a comment would not change what gets verified or what the
next engineer needs to know about the product, don't post it. Never post:

- tooling narration ("hooks blocked the push", "clippy is clean now")
- test-suite results in any form, including "full suite green"
- rebase, branch, worktree, or merge-conflict reports
- progress without an outcome ("starting the second half", "still working on this")
- flight IDs, user IDs, or "Reproduced YYYY-MM-DD" stories; that history belongs in the commit
  and the test
- anything you'd describe as being "for the record" rather than for a reader

A genuine blocker is the exception: a real comment with a real ask (what is blocked, what you
need, what you tried), and a move to **Blocked**.

### Tone

Write as an engineer. Report outcomes, not the steps you followed; first person ("Added the
delta check"); no internal workflow ("awaiting approval", "as instructed"). Every agent posts as
the account owner, so never refer to the owner in the third person, and never expose agent
tooling or its limits ("I couldn't fetch that").

---

## Commands

### Filing an issue is two steps

`gh issue create` does **not** put the issue on the board, and an issue that isn't on the board
doesn't exist as work: nobody triages it. `gh project item-add` then adds it with **no Status**, so
it sits in no column at all. Filing is: create, add to Project 7, set Status to **Triaging**.

```bash
gh issue create --repo VATUSA/OIS \
  --title "FCA metering uses cruise speed during descent, so arrival ETAs run early" \
  --body-file issue.md \
  --label "type:bug,area:flow,priority:high"

# then, as separate commands
gh project item-add 7 --owner VATUSA --url https://github.com/VATUSA/OIS/issues/<n>
.claude/scripts/board-status.sh <n> "Triaging"
```

- **The item isn't queryable the instant `item-add` returns.** If `board-status.sh` says the issue
  isn't on the board straight afterwards, wait and re-run the move as its own call. Never re-run
  `item-add`; that's how an issue lands on the board twice.
- **Read the status back** (the `projectItems` query in
  [Referring to an issue](#referring-to-an-issue)) before you report the issue as filed.
  `item-add` prints nothing on success, and a printed "moved" line is not the resource.

### Moving a card

Use `.claude/scripts/board-status.sh <n> "<Status>"`. It resolves the card from the issue's own
project items instead of listing the board, which keeps it clear of the Projects secondary rate
limit and of listing truncation. Re-read the card's status immediately before a move; a listing
taken minutes earlier has overwritten another agent's move.

### The API budget is shared

Every agent and tool on the account shares one GitHub API budget, and the Projects API has a
secondary limit that `gh api rate_limit` doesn't show. Repeated `gh project item-list` calls and
`gh pr checks` watch loops have locked `gh project` out for an hour.

- Read a card once per transition, through its own project items. Prefer local git
  (`git branch -r`, `git log origin/<branch>`) when the answer is in the refs.
- Poll CI a handful of times at most; the local gate is the primary evidence.
- When GraphQL is throttled, REST still works for everything except the board move. Post the
  comment through REST and retry the move later:

  ```bash
  python3 -c "import json; json.dump({'body': open('comment.md').read()}, open('c.json', 'w'))"
  gh api --method POST repos/VATUSA/OIS/issues/<n>/comments --input c.json --jq .html_url
  ```

### Searching for duplicates

Search the entity, not your phrasing: the symbol, file, route, or test name. Two descriptions of
one defect rarely share a verb. Run two or three narrow searches rather than one long one:

```bash
gh issue list --repo VATUSA/OIS --state all --search "trajectory descent" --limit 30
gh issue list --repo VATUSA/OIS --state all --search "metering ETA" --limit 30
```

Two traps make a search report "no duplicates" falsely:

- **`gh search issues` has no `--state all`.** It accepts only `open` or `closed`, the error goes
  to stderr, and a pipeline then prints nothing, which reads exactly like "no matches". Use
  `gh issue list --state all --search`, or the REST list
  (`gh api "repos/VATUSA/OIS/issues?state=all&per_page=100"`), and run a control query for a term
  you know exists.
- **Search lags new issues.** The index can miss an issue filed minutes ago (#728 duplicated #727,
  filed 18 minutes earlier). Also list recent issues directly, which reads the database rather
  than the index: `gh issue list --repo VATUSA/OIS --state all --limit 30`.

When you find a duplicate, don't drop your finding: comment onto the existing issue whatever yours
establishes that it doesn't, and don't reopen, relabel, or reassign it.

### Adding a label

The taxonomy above exists on the repo. To add a label to it:

```bash
gh label create "area:example" --repo VATUSA/OIS --color 1f77b4 --description "What it covers"
```
