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

Every issue carries **one `type:`, one `area:`, one `priority:`**, plus any modifiers. (The repo
currently has only GitHub's default labels — the taxonomy below needs to be created once; see
"Creating the labels" at the end.)

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

A title is a one-line statement of the problem, specific enough to tell apart from its neighbours —
ideally **symptom → consequence**. Name the real thing, not the fix.

- ✅ `FCA metering uses cruise speed during descent, so arrival ETAs run early`
- ✅ `Adding a permission without the catalog entry 400s the access editor`
- ❌ `Fix metering` · ❌ `ETA bug` · ❌ `Improve permissions`

---

## Body structure

Use these sections (drop ones that don't apply — e.g. a pure feature has no "Why the neighbouring
path is fine"). Ground every claim in evidence: real file:line references, real captured output, not
"it should."

```markdown
## What happens
The observable problem. Point at the exact code — `path/to/file.rs:123` — and show it. For a bug,
prove it with real output/behavior, not a description of it.

## Why
The mechanism / root cause. Which line, why the guard doesn't fire, what the wrong value is.

## Why the neighbouring path is fine   ← for bugs, when relevant
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
Data path: <source → feed module or handler → repo → API route → consumer (hook → page, or the
bot), real files and routes only; the mode when it matters; or "none">
Pre-existing; found while <what you were doing>.   ← provenance, when it's incidental
Relates to #N / Duplicate of #N.                    ← after a duplicate search
```

Three OIS-specific habits in that footer:

- **Blast radius.** OIS has a few changes that reach further than they look — the single
  trajectory/ETA model in `feed/trajectory.rs` (three callers), the permission/role
  "three-places-in-sync" invariants, and the OpenAPI→client contract (needs a regen). If the issue
  touches one, say so; if it touches none, say "none." Blast radius says *how far* a change
  reaches; Data path says *how the data gets there*.
- **Data path.** Trace the chain the issue concerns, in the direction the data moves, so a reviewer
  doesn't re-derive it and a check can be reproduced. Name only hops that exist — a file, a
  `module::function`, a route as written in `backend/src/router.rs`, a hook, a page — and skip any
  hop the path doesn't have. In these examples backend paths are relative to `backend/src/` and web
  paths to `web/src/`; in an issue, either form is fine as long as it resolves. For example:
  - Live arrival flow: `VATSIM datafeed (feed/vatsim.rs) → handlers/feed.rs::airport_flow →
    feed/flow.rs::compute (program + CFRs from repos/tmu.rs) → GET /api/v1/tmu/flow/{icao} →
    useAirportFlow (web/src/lib/feed.ts) → pages/airport.tsx`
  - Its historical twin: `stats.flight + stats.position →
    feed/stats/reconstruct.rs::reconstruct_at → handlers/stats.rs::hist_flow →
    feed/flow.rs::compute → GET /api/v1/stats/hist/flow/{icao} → useAirportFlow inside
    HistoricalProvider → features/dashboard/view-widgets.tsx → pages/stats/dashboard.tsx`
  - A write: `pages/admin/access-control.tsx → useSaveUserAccess (web/src/lib/access.ts) →
    POST /api/v1/admin/users/{cid}/access → handlers/access.rs::update_user_access →
    repos/access.rs`

  **Name the mode** when the change, or a check of it, behaves differently across modes; otherwise
  leave it out:
  - *Live vs historical.* Historical ("time-machine") mode is the scrubber instant a
    `HistoricalProvider` (`web/src/lib/historical-context.tsx`) supplies; today only the dashboard
    replay (`pages/stats/dashboard.tsx`) mounts one. Any hook or component that reads
    `useHistoricalAt()` switches with it — grep for it in `web/src` rather than trusting a list;
    `useAirportFlow` above is one. Given an instant, the data hooks call `/api/v1/stats/hist/*`
    instead of the live route; the `useMode*` hooks in `web/src/lib/historical.ts` take that
    instant as an explicit `at` argument from their callers (`features/dashboard/sources.ts`).
    If the issue lives on one side, say which; if it's in shared compute (`feed/flow.rs::compute`
    above), both twins are affected — say so.
  - *Web vs desktop.* The desktop app (`desktop/src-tauri`) renders the same `web/` bundle, but
    signs in with a token (`POST /api/v1/auth/desktop/exchange` and `/refresh`) and has its own
    origins and CSP. Name it when the path crosses auth, CORS/origins, CSP, or a Tauri command.

  Write `Data path: none` for docs, tooling, CI, and other changes that move no runtime data.
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
> Blast radius: permissions (three-in-sync).
> Data path: `pages/admin/access-control.tsx → useSaveUserAccess → POST
> /api/v1/admin/users/{cid}/access → handlers/access.rs::update_user_access →
> repos/access.rs::fetch_access_catalog_names`.
> Pre-existing; found while adding a flow permission.

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

**Before filing, search for a duplicate** (`gh issue list --repo VATUSA/OIS --search "keywords"`,
include closed). Link related issues (`Relates to #N`) and mark true duplicates `status`/close with a
pointer rather than filing again.

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
- **Comment sparingly** — an issue is a spec, not a chat log. Comment only at real moments: picking
  it up, hitting a genuine blocker (say what and why), or finishing (what changed + the verifying
  test). No running narration, no per-attempt logs, no flight/user IDs or "Reproduced YYYY-MM-DD"
  stories — that history belongs in the commit and the test.
- **Attribution:** an agent-drafted issue or comment ends with a `🤖 Drafted by Claude Code` line so
  its origin is clear.
- **Other repos are read-only.** Reference AvioDeck (or any other repo) for patterns, but never
  create, edit, comment on, or label issues outside `VATUSA/OIS`.

---

## Commands

Create an issue with labels and drop it on the board:

```bash
gh issue create --repo VATUSA/OIS \
  --title "FCA metering uses cruise speed during descent, so arrival ETAs run early" \
  --body-file issue.md \
  --label "type:bug,area:flow,priority:high"

# add it to the board (project 7)
gh project item-add 7 --owner VATUSA --url <issue-url>
```

Duplicate search before filing:

```bash
gh issue list --repo VATUSA/OIS --state all --search "metering descent eta"
```

### Creating the labels (one-time)

The repo still has GitHub's default labels. Create the taxonomy above once, e.g.:

```bash
gh label create "type:bug"        --repo VATUSA/OIS --color d73a4a --description "Behaves wrong"
gh label create "area:flow"       --repo VATUSA/OIS --color 1f77b4 --description "FCAs, metering, runway, trajectory"
gh label create "priority:high"   --repo VATUSA/OIS --color b60205 --description "Major/blocking; next up"
gh label create "technical-debt"  --repo VATUSA/OIS --color d4c5f9 --description "Pre-existing, filed per the scope tests"
# …and the rest of type:/area:/priority: from the tables above
```
