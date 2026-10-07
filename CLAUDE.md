# CLAUDE.md

Guidance for Claude Code in the OIS repository.

**Read [`AGENTS.md`](AGENTS.md) first — it is the source of truth** for the project overview,
architecture, the sync-invariants, commands, testing, and conventions. This file adds only the
Claude-Code-specific layer on top. Canonical detail lives in `AGENTS.md`; when a fact belongs to
both, it goes there and this file points at it. Two copies drift — don't paste `AGENTS.md` detail in
here.

---

## The short version

OIS is VATUSA's operations platform: a **Rust/Axum backend** (the authority for identity,
permissions, and every domain) over Postgres, a **Vite + React web app** consuming an
OpenAPI-generated typed client, and a **Rust Discord bot** (built — it drains the backend's
outbound-job queue). One monorepo, Cargo + pnpm/Turbo workspaces, tied together by a `justfile`.
See `AGENTS.md` § Architecture.

---

## Standing working agreements

These are project rules, not preferences:

- **Work on `next`. Never create or switch branches in the primary checkout.** `next` is the
  integration branch — issue work forks a worktree from `origin/next` and PRs target `next` (see
  `.claude/commands/start.md`/`ship.md`); a worktree is the sanctioned exception to "never create or
  switch branches." `main` is promoted from `next` via a separate, manual release PR only — don't
  target `main` directly for issue work.
- **Always produce a commit message** for a completed unit of work — conventional-commit style
  (`type(scope): summary`), a short body, and a `Closes #N` line when it maps to an issue. Commit/push
  only when the user asks. **No AI attribution, anywhere**: the rule (commits, PRs, issues, comments)
  is stated once, in `AGENTS.md` § Git workflow.
- **Regenerate the typed client after any contract change.** Editing an endpoint or a
  `#[derive(ToSchema)]` model and *not* regenerating leaves the web typecheck compiling against a
  stale contract — a silent failure. See `AGENTS.md` § "The API contract → typed client".
- **Verify before "done": run `just ci-full`**, which mirrors CI (`just ci` is the fast subset; it
  skips clippy, vitest, the audits and client drift — see `AGENTS.md` § Commands). For a contract
  change, regenerate the client first. For DB behavior, run the stack and exercise the endpoint. Once
  the PR is open, read its check-runs before calling CI green (`AGENTS.md` § Testing & verification).
- **Filing issues** follows [`docs/github-issues.md`](docs/github-issues.md) (labels, body structure,
  scope tests, board). Don't self-assign/close/merge; keep comments to real moments; other repos are
  read-only.
- **Any UI work follows [`DESIGN.md`](DESIGN.md).** OIS is one dark operator-console system — one
  accent, no gradients, no chrome shadows, hairlines, continuous corners, the 400/600/700 ladder,
  tokens only. Restyle through the shared shell/components in `packages/ui`; never hand-style a one-off
  screen. `DESIGN.md`'s principles come from `.claude/skills/claude-apple-design-system/`.

## The invariants that bite

From `AGENTS.md`, repeated here only as a checklist because missing one fails silently:

- A **new permission** → marker in `auth/permissions.rs` + string in `crates/ois-core/src/catalog.rs`
  + `insert into access.permissions` migration. All three.
- A **new assignable role** → `access.roles` migration + `default_roles()` + `ASSIGNABLE_USER_ROLES`.
- A **new endpoint** → `router.rs` route + `openapi.rs` path (+ schema) + regenerated client.
- The **trajectory model** (`feed/trajectory.rs`) is shared by every ETA surface — a change reaches
  all of its callers (`AGENTS.md` § The trajectory / ETA model lists them); verify each.
- The **feed subsystem has no DB handle** — it reads `AppState` `ArcSwap` caches, never queries
  inline. New feed-visible config follows the "cache + refresh job + force-reload on write" pattern.

## Rules (`.claude/rules/`)

Review, test, git, lifecycle, and prose standards live in `.claude/rules/`. Seven load in every
session; `rust-backend`, `secure-coding`, `database-postgres`, and `web-frontend` have `paths:`
frontmatter and load only when Claude reads or edits a matching file. Rules link to `AGENTS.md`.

## Tool selection

- Use **Explore / Task** for open-ended understanding — "how does metering work?", tracing a call
  path across `handlers → repos → feed`, or mapping which of the trajectory model's callers a change
  hits. If you're about to answer "how does X work" after only grepping, stop and Explore instead.
- Use **Grep / Glob / Read** for a specific needle you can already name — a struct, a route, a
  migration. You know the layout from `AGENTS.md`; jump straight to the file.
- Prefer the dedicated file tools over shell `cat`/`sed`; run independent reads/searches in parallel.

## When to ask vs. act

Ask when a requirement is ambiguous, several valid approaches exist, or you'd be baking in a
business-logic or architectural assumption — several issues this codebase carries are intentionally
under-specified and want a decision, not a guess. Act autonomously when a sibling pattern exists to
mirror exactly (a new config domain mirrors `airport_configs`; a new admin page mirrors an existing
one) or the bug and fix are unambiguous. If you're guessing, ask.

---

*Everything else — the full architecture, the permission model, `just` commands, testing, migrations,
env vars — is in [`AGENTS.md`](AGENTS.md).*
