---
description: Formal review of the committed HEAD before shipping — gates, code quality, and OIS pitfall scan.
---

You are performing a formal review before shipping. Review the **committed HEAD** of the feature
branch (commit first — a dirty worktree means uncommitted changes were never reviewed). Work through
every phase; all must pass before you report the review clean.

## Phase 0 — Pin the reviewed commit
`git rev-parse HEAD` — this is the reviewed SHA. If a finding needs a code change, apply it, commit,
re-pin, and re-run from Phase 1 (the review is of a commit, not a dirty tree).

## Phase 1 — Diff inventory
`git diff main...HEAD --stat`, then read the full diff. Know exactly what was added, changed, removed.

## Phase 2 — Code-quality review
Review every changed file for:
- **Correctness** — logic errors, off-by-one, `unwrap`/`expect` on fallible paths, races, null/None safety.
- **Bad code** — poor names, needless complexity, SOLID violations, dead code introduced.
- **Security** — SQL built by string-interpolating user input (sqlx queries must **bind** params);
  a **mutation handler missing its `RequirePermission<P>`**; data-dependent checks (ownership,
  ARTCC scope) skipped; secrets/logging leaks.
- **Missing tests** — new logic (metering, trajectory, permission resolution) without a unit test;
  untested sad paths.
- **Performance** — heavy CPU or a blocking DB call on the async workers (route resolution / metering
  must run under `spawn_blocking`); N+1 queries; work inside the feed poller that should be cached.

Fix what you find. **Ask me before**: changing a test's expected behavior; altering business logic
or a calculation; creating or editing a migration; removing/renaming a public API or permission;
anything user-visible. **Auto-fix without asking**: typos, formatting, obvious missing error
handling, doc comments.

## Phase 3 — Gates (mirror CI)
Run and make green, in the changed workspaces:
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace` (read `test result:`, not the exit code)
- If the API contract moved (an endpoint or a `#[derive(ToSchema)]` model): **regenerate the client
  first** — `OIS_OPENAPI_URL=<running backend>/docs/api/v1/openapi.json pnpm --filter @ois/api-client codegen` — then `pnpm typecheck`.

`just ci` runs the whole set; use it unless you're iterating on one gate.

## Phase 4 — OIS pitfall scan (grep the changed files, real `file:line`)
These have bitten before and are cheap to catch here:
- A **new permission** with a `permission!` marker or `access.permissions` row but **no
  `catalog.rs` entry** (or vice-versa) — the three must be in sync.
- A **new role** missing from any of: the `access.roles` migration, `default_roles()`,
  `ASSIGNABLE_USER_ROLES`.
- A **new handler** wired into `router.rs` but not `openapi.rs` (or vice-versa) — it won't reach the
  generated client.
- A **`#[derive(ToSchema)]`** change without a client regen.
- **SQL in a handler** — it belongs in `repos/`.
- A **migration edited in place** — migrations are append-only; add a new numbered one.
- A `RequirePermission` **absent** from a route that mutates state.

## Phase 5 — Report
Confirm every gate is green and list what you fixed vs. what you're flagging to me. OIS has no marker
files — the evidence is the green gates and this write-up. Only CRITICAL/MAJOR findings must be
remediated; note nits without acting on them.
