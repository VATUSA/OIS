# AGENTS.md

Guidance for any AI agent working in the OIS repository. This is the **canonical** agent guide —
`CLAUDE.md` layers Claude-Code-specific notes on top and points here for the detail. Keep the two
in sync by putting shared facts here and pointing, not by copying.

When a convention here conflicts with intuition, follow the convention. When something here
conflicts with the code, the code wins — fix the doc in the same change.

---

## Project overview

**OIS** (Event Operational Information System) is VATUSA's in-house operations platform: event
operations, traffic management (superseding the third-party `vatflow.io`), ACE support, and
fine-grained access control. It **works alongside** the existing VATUSA website (which keeps event
creation/review/posting). The permission core and service architecture are ported from **osmium**
(the vZDC backend) and scaled to national, multi-ARTCC operation.

One monorepo, four parts, one API:

| Path | Stack | What it is |
| --- | --- | --- |
| `backend/` | Rust · Axum | The API and the authority for identity, permissions, and every domain. Postgres + sqlx, embedded migrations, self-served OpenAPI. |
| `discord/` | Rust · serenity | The bot. Owns no data — leases jobs from `integration.outbound_jobs` via `POST /api/v1/integration/jobs/lease`, performs the Discord side-effect, then acks with the ids it produced. Its button and modal interactions call back through the same API as a service account. |
| `web/` | Vite · React | The site. TanStack Router + Query, consuming the OpenAPI-generated typed client. |
| `desktop/` | Tauri · Rust | The native app. Not a second frontend — a webview shell (`src-tauri`) around the **same** `web/` bundle. *(shipping signed auto-updating builds; desktop-only features in progress)* |
| `crates/ois-core` | Rust | DB-free domain + permission types (ported from osmium). |
| `crates/ois-client` | Rust | Typed backend client used by the bot. |
| `packages/api-client` | TypeScript | OpenAPI-generated client (`@ois/api-client`) for web + desktop. |
| `packages/ui` | TypeScript | Shared shadcn/ui components + theme. |

Two workspace managers coexist: a Cargo workspace (`backend`, `discord`, `crates/*`,
`desktop/src-tauri`) and a pnpm + Turborepo workspace (`web`, `desktop`, `packages/*`). The root
`justfile` ties cross-language tasks together. Design docs live in `docs/` (architecture +
per-feature specs); user docs are a VitePress site in `docs-site/`. The backlog is
`docs/IDEAS.md`; the phased plan is `docs/PLAN.md`.

Rust: edition 2024, MSRV 1.85, **nightly** toolchain (for `-Zthreads` — no nightly *language*
features, so `stable` remains a valid fallback).

**On Linux, `just ci` needs the GTK/WebKit dev packages.** `just check` is
`cargo check --workspace`, and the workspace includes the Tauri shell (`desktop/src-tauri`), whose
Linux backend will not even `cargo check` without them — so a backend-only change still fails
without this. macOS and Windows use the OS webview and need nothing extra.

```bash
sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev
```

---

## Architecture

### Backend shape (`backend/src/`)

A single Axum binary over one Postgres database with **per-domain schemas** (`platform`, `identity`,
`access`, `org`, `events`, `tmu`, `ace`, `flow`, `integration`, `stats`, `media`, `web`).

```
handlers/   thin HTTP handlers, one module per API domain — no SQL here
repos/      the SQL layer — ALL queries live here; handlers stay thin
models/     request/response + row types (serde + utoipa + sqlx::FromRow)
auth/       RequirePermission extractor, principal resolution, permission markers
jobs.rs     background workers (nav/winds refresh, lifecycle, cleanup, compaction…)
job_registry.rs  in-memory job status + manual-trigger registry
metrics.rs  Prometheus registry + the HTTP-metrics layer; GET /metrics projects AppState
feed/       live VATSIM-feed subsystem (poller, nav, airports, trajectory, flow, runway…)
realtime.rs in-process broadcast hub behind GET /api/v1/ws
audit.rs    middleware that records every successful mutation
state.rs    AppState — DB pool + feed + nav/winds/profiles caches + realtime hub + job registry
router.rs   every route; openapi.rs registers paths + schemas
```

Request flow: **handler → repo → model**. Handlers stay thin; if you write SQL in a handler, move
it to a repo. Versioned REST under `/api/v1`; OpenAPI at `/docs/api/v1/openapi.json` + Swagger UI at
`/docs/swagger`.

### Permissions (the heart of the platform)

Explicit, path-based `segments.action` strings (e.g. `tmu.tmi.publish`). **Nothing is implied by
role name.** The terminal action is one of a fixed verb set: `read, create, update, delete, publish,
assign, decide, request, approve, deny, claim`. Grants can be national or **scoped to an ARTCC**
(`artcc_id` on the grant; `NULL` = national). Deny beats allow. `SERVER_ADMIN` is a bootstrapped
singleton (env CID) holding everything implicitly.

Enforcement is a typed **`RequirePermission<P>`** Axum extractor keyed by a marker type. Declaring
it in a handler's argument list is the *only* way to satisfy it, so a missing check is a visible gap
in the signature, not a silent omission. Data-dependent checks (ownership, "request is open", ARTCC
scope via `access_repo::permission_scope(...).allows(...)`) are done in the handler **on top of** the
extractor.

**A new permission lives in three places that MUST stay in sync:**
1. `permission!(MarkerName, ["segments"], Action)` in `backend/src/auth/permissions.rs`
2. the string in `crates/ois-core/src/catalog.rs` (the editor's catalog)
3. an `insert into access.permissions (...)` in a new migration

**A new assignable role also lives in three places:**
1. `insert into access.roles (...)` in a migration
2. `default_roles()` in `crates/ois-core/src/catalog.rs`
3. `ASSIGNABLE_USER_ROLES` in `backend/src/repos/access.rs`

Baseline access every signed-in member gets is the **`USER` group** (`BASELINE_ROLE` in
`backend/src/handlers/auth.rs`), granted at sign-in. Its content is the group's
`access.role_permissions` rows (seeded by `0094_seed_role_permissions.sql`), so changing the baseline
needs no backfill (#544).

### The API contract → typed client

The backend emits OpenAPI (utoipa). The web/desktop client is **generated** from it, so the contract
is the single source of truth for types — there are no hand-written client models.

**After changing any endpoint or a `#[derive(ToSchema)]` model, you MUST regenerate the client**,
or the web typecheck compiles against a stale contract:

```bash
# with a backend running (e.g. on :3001 to avoid clashing with a dev :3000):
OIS_OPENAPI_URL=http://127.0.0.1:3001/docs/api/v1/openapi.json \
  pnpm --filter @ois/api-client codegen
```

New handlers must be registered in **both** `router.rs` (the route) and `openapi.rs` (the path +
any new schema), or they won't appear in the generated client.

**A `///` doc comment on a `ToSchema` model or field is contract, not prose** (#591): utoipa emits it as the
schema `description`, so editing it changes the OpenAPI document. Regenerate the client, or only CI's
`client-drift` will notice. From 1.0, an edit on the supported surface (`docs/architecture/api-surface.md`)
is a contract edit and gets a changelog entry like any other.

**Record contract changes** in `docs-site/reference/api-changelog.md` (#591). Before 1.0 the entries are
informational; nothing is stable.

The one deliberate exception is **`GET /metrics`** (#382): Prometheus text exposition, not JSON the
SPA consumes, so it is registered in `router.rs` only and there is **no client regen** for it.
Adding another endpoint outside the spec needs the same kind of justification — "the generated
client could not use it if it wanted to" — not merely "the SPA doesn't call it yet".

### The trajectory / ETA model (one predictor, many callers)

`backend/src/feed/trajectory.rs` is the **single** ETA predictor — a vertical-profile integrator
(climb/cruise/descent schedules, ISA Mach↔TAS, top-of-descent, service-ceiling cap) plus configurable
per-aircraft `AircraftProfile`s resolved by exact type → wake class → default. Its callers:

- **Run the model:** FCA metering (`handlers/flow.rs`), airport-flow demand (`feed/flow.rs`), runway
  ETE (`feed/runway.rs`), sector occupancy (`feed/sector_tracks.rs`), and the shared prediction
  service (`feed/predict.rs`), which builds the `VerticalProfile` the others time against.
- **Inputs, not callers:** `feed/fca.rs` supplies the route `predict.rs` times along
  (`fca::route_path`), and `feed/taxi_estimate.rs` supplies the ground allowance `feed/flow.rs` adds
  before wheels-up. A `trajectory.rs` change does not reach them, but a change to them moves every ETA
  built on them.
- **Carry the `ProfileTable` only:** `state.rs` (the cache), `jobs.rs` (its refresh),
  `repos/aircraft_profiles.rs` (loading it), `handlers/aircraft_profiles.rs` (reloading it on write)
  and `scope_test_support.rs` (a test fixture).

A change here reaches every caller above — verify each, don't reason about one. The list goes stale:
re-derive it with `git grep 'trajectory::' -- backend/src` and `git grep 'predict::' -- backend/src`
before relying on it, and keep only non-test call sites: both also match doc comments,
`#[cfg(test)]` code and constant imports.

### The live feed subsystem (`feed/`)

A background poller keeps an in-memory VATSIM snapshot (plus nav/airport/runway/winds/profile
caches). **Feed functions have no DB handle** — they read the caches held in `AppState` behind
`ArcSwap` (lock-free). Anything the feed needs from the DB must be loaded into an `AppState` cache by
a background job and read from there, never queried inline in the feed.

### Realtime

`AppState::publish(topic)` fans a small "something changed" nudge (no payload) to every client on
`GET /api/v1/ws`; the web maps topic → React-Query invalidation in `web/src/lib/realtime.ts`. REST is
the single source of truth; the socket only signals, and the app degrades to polling if it drops. A
mutation that changes flow/TMU state should publish the matching topic.

### Auditing

`audit.rs` middleware records every successful mutation to `access.audit_logs`. Access changes carry
a **required human reason** plus before/after snapshots.

### Web

Vite + React, TanStack Router (`web/src/router.tsx`) + Query. Data hooks in `web/src/lib/*` wrap the
generated `ois` client. shadcn/ui components live in `packages/ui` (`@ois/ui`). User-facing settings
are declarative: add an entry to `web/src/lib/settings/registry.ts` and it appears on `/settings`;
read it anywhere with `useSetting(key, default).value`.

**Design language — read [`DESIGN.md`](DESIGN.md) before building or restyling any UI.** OIS has one
system (a quiet dark operator console: one accent, no gradients, no chrome shadows, hairlines,
continuous corners, a 400/600/700 type ladder, tokens only). `DESIGN.md` is the concrete app spec; its
principles come from the Apple-inspired skill at `.claude/skills/claude-apple-design-system/`. Build the
shell + shared components once (in `packages/ui`) and render every screen through them — don't hand-style
one-offs.

---

## Working rules

### Zero-tolerance (no exceptions)

1. **No assumptions.** If you're guessing at a requirement or a behavior, STOP and ask, or go read
   the resolved value / real bytes. A config *default* is not the configured value.
2. **No scaffolding reported as done.** A migration, model, or permission string is not a feature
   until something calls it end-to-end. A new capability isn't done until every layer it needs
   exists — for an endpoint that means handler + route + `openapi.rs` + regenerated client + UI +
   verification. Grep for the usage before claiming complete.
3. **Keep the sync-invariants intact.** Adding a permission or role touches the three places listed
   above. Adding a `ToSchema` model or endpoint means regenerating the client. Miss one and the
   build or the contract silently drifts.
4. **Trace before you fix.** Follow the real path (handler → repo → DB, or dispatch → job → cache)
   and name the step that failed before proposing a change. No sweep/retry/fallback to mask a
   symptom while the primary path stays broken.
5. **No unverified completions.** Don't report "done" without running the relevant checks (see
   below). Tests passing you wrote against inputs you wrote only prove the two agree — prefer
   evidence you didn't author (a value printed from the running app, a real OpenAPI diff).

### Before shipping a change, ask what would make it wrong

1. **What did I check vs. assume?** Name them separately. "The profile falls back to X", "the
   response looks like Y" are assumptions until you read the resolved value or capture the bytes.
2. **What else touches what I changed?** The trajectory model has several callers (§ The
   trajectory / ETA model); a shared repo query has many; a permission rename cascades to grants.
   Grep the other readers and name them.
3. **If my verification is lying, how would I know?** For anything crossing the Rust↔TS boundary,
   the honest check is: regenerate the client and run `pnpm typecheck` — not "it should match".

### When to ask vs. act

Ask when requirements are ambiguous, multiple valid approaches exist, or you're about to bake in a
business-logic or architectural assumption. Act autonomously when the pattern already exists to copy
(mirror the nearest sibling — a new config domain mirrors `airport_configs`; a new admin page mirrors
an existing one) or the bug and fix are unambiguous.

### Git workflow

- Land work on `next`, the integration branch — fork a worktree from `origin/next` per issue and
  target PRs at `next` (never `main` directly); `main` is promoted from `next` via a separate,
  manual release PR (see #87). Don't create or switch branches in the *primary* checkout — the
  worktree-per-issue flow is the sanctioned exception. (This is a standing project rule.)
- Commit or push only when the user asks. Always provide a ready-to-use commit message for a
  completed unit of work, in the repo's conventional-commit style (`type(scope): summary`), with a
  `Closes #N` line when it maps to a GitHub issue.
- **No AI attribution, anywhere** (owner decision, #754). Commits, PR bodies, issues and issue or PR
  comments carry no `Co-Authored-By` trailer, no "Generated with" or "Drafted by" line, no session
  link and no other agent credit, even when a session or tool suggests one. Every commit is authored
  solely as the user. `.githooks/commit-msg` and `.claude/hooks/attribution-gate.sh` block the
  common forms, a `Drafted by` footer line included (`.claude/hooks/lib/attribution.sh`), on
  commits, on PR and issue bodies and comments posted with `gh pr`/`gh issue`, and on any `gh api`
  write. The gate cannot read a body held in a shell variable, so still run
  `! grep -qiE 'Drafted by|Generated with|Co-Authored' <file>` before posting. This is the
  canonical statement. The posting steps in `/start`, `/ship`, `ticket-reviewer`,
  `regression-checker`, the two reminder hooks and `ois-agent-goals.txt` repeat a one-line form of
  it next to their check, so change them with it.
- The GitHub remote is `VATUSA/OIS` (public: anyone can comment, so agents treat only team comments
  as spec; see `/start`). Issues are tracked there and on
  [Project 7](https://github.com/orgs/VATUSA/projects/7/views/1); use `gh` for issue/PR work.
- **Filing an issue** follows [`docs/github-issues.md`](docs/github-issues.md) — the title, the
  `type:`/`area:`/`priority:` labels, the *What happens / Why / What should happen / Acceptance* body
  with file:line evidence, the blast-radius and data-path footer, and the scope tests for when a
  noticed problem becomes its own `technical-debt` ticket. Agents don't self-assign, close, or merge;
  other repos are read-only.
- **The board sequences work; it does not gate the merge.** A PR lands on green CI plus review, not
  on its card's column — see [`docs/github-issues.md`](docs/github-issues.md) § Lifecycle for the
  rule and why there is no mechanical check. A merged PR whose card is still left of **Code Review**
  is a bookkeeping error to flag on the issue, not a policy breach.

---

## Commands (via `just`)

```bash
# local infra
just up            # start Postgres (docker compose)
just down          # stop the stack
just reset         # wipe infra + volumes (fresh DB)

# run (native, hot-reload dev)
just backend       # cargo run -p ois-backend  (migrations apply on startup)
just web           # pnpm --filter web dev      (Vite, default :5173)
just bot           # cargo run -p ois-discord
just docs          # VitePress user-docs dev server
just desktop       # Tauri shell + web dev server, hot-reloaded into the webview

# rust
just check         # cargo check --workspace --all-targets
just fmt           # cargo fmt --all
just fmt-check     # cargo fmt --all -- --check
just test-rust     # cargo test --workspace --all-targets -- --test-threads=1

# js
just test-js       # pnpm test

# desktop
just desktop-build # bundle the Tauri app for the host platform

# the gates
just ci            # fast subset: migration-version check + fmt-check + cargo check + rust tests, then pnpm lint && pnpm typecheck
just ci-full       # what CI runs (run before calling anything done): adds clippy -D warnings, nextest, doc tests,
                   #   pnpm test, pnpm audit, cargo deny and the client-drift check; prints PASS/FAIL/SKIPPED per step

# once per clone
just setup         # git hooks from .githooks/: pre-commit (rustfmt, gitleaks, migration versions),
                   #   commit-msg (no AI attribution), pre-push (clippy)
```

`just ci-full` mirrors `.github/workflows/ci.yml` except the cross-platform `desktop` matrix, which
needs the other operating systems. It runs every step and fails at the end if any failed; a step it
cannot run (no `cargo-deny` installed) shows as SKIPPED, not passed. Rust tests use nextest with CI's
profile when it is installed and say so when they fall back to `cargo test`.

First-time setup: `cp .env.example .env`, fill the VATSIM OAuth block, `pnpm install`, `just setup`.
`.env` is read by the backend (`dotenvy`) and docker-compose; Vite reads `web/.env.local`. Both are
gitignored.

**Agent gates.** `.claude/settings.json` wires Claude Code hooks from `.claude/hooks/`, always run
from the primary checkout's copy, so a branch cannot loosen its own gates. They block a push to
`main`/`next`, `gh pr merge`, a branch created or switched in the primary checkout, `git add -A`/`.`,
a branch name off the `{feat|fix|chore}/{issue}/{desc}` pattern, AI attribution in a commit or PR,
`gh pr create` on a HEAD with no `/review-before-shipping` marker, and a staged migration whose number
another origin branch already took. `bash .claude/hooks/test/run.sh` tests them.
`.claude/scripts/review-scan.sh [base]` scans the committed diff for broken sync-invariants (see
§ Architecture), SQL or `unwrap` in handlers, and an applied migration edited in place.

---

## Testing & verification

- **Rust tests are self-contained** (no shared global state). CI runs them in parallel under
  `cargo nextest run`; locally use `just test-rust` (`justfile`), because threads sharing one
  `cargo test` process collide in the sqlx test harness. Pure logic (the trajectory model, permission tree, metering) has unit tests; add tests
  alongside such code.
- **Never hit real external APIs in tests** (VATSIM, VATUSA, Open-Meteo, AWC). The feed and clients
  are structured so the pure logic is testable without the network.
- **The web gates are `pnpm lint`, `pnpm typecheck` and `pnpm test`** (vitest in `web` and
  `packages/ui`; `just ci` skips it, `just ci-full` runs it). Lint is ESLint (root `eslint.config.mjs`)
  over `web` and `@ois/ui`: `react-hooks/rules-of-hooks` and `@typescript-eslint/no-unused-vars` are
  errors, and so is `react-hooks/exhaustive-deps` (since #329). Typecheck only tells the truth after the client
  is regenerated for any contract change (see codegen above).
- **DB-touching repo logic** can be covered by a `#[sqlx::test]` (real Postgres, one throwaway
  database per test, migrations applied automatically — no `migrations = "..."` attribute needed,
  it auto-discovers `backend/migrations`). CI provisions a `postgres:17` service for the `rust` job
  and exports `DATABASE_URL` to it; to run the same tests locally, point `DATABASE_URL` at your dev
  Postgres (`just up` starts it) and run `just test-rust` — no other setup. Since the repo
  layer uses runtime queries (no compile-time `sqlx` macros), the DB need not be present to
  *compile*, only to *run* a `#[sqlx::test]`. Handler-level / end-to-end behavior is still generally
  verified by running the full stack (`just up && just backend`) and exercising the endpoint.
- Read the `test result:` summary line, not just the exit code.

Definition of done for a change: `just ci-full` is green, with any SKIPPED step named in the PR (see
§ Commands); once the PR is open, its check-runs have been read (`gh pr checks <n>`, once, pending
included) and reported as they stand, since CI is the verdict and the local gate is not; the client
is regenerated if the contract moved; and any DB migration has been applied (it applies
automatically on the next backend start — it is idempotent-friendly and numbered sequentially).

---

## Conventions & gotchas

- **Migrations** are `backend/migrations/NNNN_name.sql`, embedded via `sqlx::migrate!` and applied on
  startup. Never renumber or edit an *applied* migration — add a new one. Text UUID PKs
  (`gen_random_uuid()::text`), `created_at`/`updated_at` timestamptz with a
  `platform.touch_updated_at()` trigger, check-constrained status enums, FK cascade where a child
  can't outlive its parent.
- **Picking a migration number is contended while several PRs are open.** Each picks "the next free
  number" against a view that goes stale, and a duplicate does not fail cleanly: sqlx applies both
  files and the second violates `_sqlx_migrations`' primary key, leaving the database half-migrated
  (#569). Pick a number **above the highest on `next` _and_ in every open PR** — this lists the open
  PRs' claims:

  ```bash
  gh pr list --state open --json number --jq '.[].number' | xargs -I{} gh pr view {} --json files --jq '.files[].path' | grep -o 'migrations/[0-9]*' | sort -u
  ```

  A gap in the sequence is harmless; a repeat is fatal. If your unmerged PR collides, renumber it
  *upward* past every open claim — not into a gap someone else may also take.

  Three guards catch a duplicate, and they fail at different moments. The
  `migration_versions_are_unique` test (`backend/src/lib.rs`) names the colliding files as soon as
  both are in one tree, needing no database. `just check-migrations` (part of `just ci`) and the
  `migrations` CI job run the same check against the PR's merge ref, so GitHub's "merge into base"
  view catches a collision the branch alone cannot see. The image build then `needs: migrations`, so
  a duplicate that slipped in behind a later merge can never be tagged and deployed. None of them can
  see across two open branches, so the number you pick is still yours to get right.
- **Config that must reach the feed** (aircraft profiles, e.g.) is cached in `AppState` behind
  `ArcSwap` and refreshed by a `jobs.rs` worker; the write handler also force-reloads the cache so
  edits apply immediately. Mirror that pattern for any new feed-visible config.
- **Errors** use a uniform envelope with stable codes: 400 `bad_request`, 401 `unauthorized`,
  403 `forbidden`, 404 `not_found`, 409 `conflict`, 503 `service_unavailable`. Repos map DB failures
  to `ApiError`; handlers return `Result<Json<T>, ApiError>`.
- **Auth**: VATSIM Connect OAuth is the only human sign-in (no local passwords). Bearer tokens are
  user API keys (`ois_pat_…`, capped to the owner's live access) or service accounts (`ois_sa_…`).
  The VATUSA roster webhook is authenticated by HMAC over the body, no session/bearer.
- **Blocked subsystems**: transactional email is designed-only — features needing it (reminder
  emails) can't be finished until it is built; say so rather than stubbing. The Discord bot is
  **not** in this category: it runs, drains the outbound-job queue, and sends DMs. Its one gap is
  slash commands — it registers none and needs no privileged intents, so anything phrased as a chat
  command needs that built first.
- **Comments and commit bodies** describe the code as it is now, for a future reader — not a
  per-change narrative with dates/IDs. That history belongs in the commit message and the test.

---

## Versioning

Three independent axes — they are not the same number:

- **Product version** — the root `VERSION` file, SemVer, the single source of truth for the
  deployed app. It already flows to the backend binary (`backend/build.rs`, baked in as
  `OIS_VERSION`) and the web footer/image tags (`web/vite.config.ts`, `build-images.yml`);
  `backend/Cargo.toml` and `web/package.json` mirror it manually on each bump (neither is read by
  anything at runtime — they're just Cargo/npm's required version string). Pre-1.0 (`0.y.z`) means
  no stability guarantee — anything, including the API shape, may change between `0.y` releases.
  Reaching `1.0.0` is a deliberate commitment: we stand behind `/api/v1` and won't break it.
- **API contract version** — the `/api/v1` URL path. Decoupled from the product version; only
  moves on a breaking change, and only once frozen — a breaking change then means a new `/api/v2`
  served alongside a deprecation window. Pre-1.0, `v1` stays malleable: breaking changes are
  allowed on it (regen the typed client, move internal callers) rather than spinning up `/v2` for
  every early change, since only the internal `@ois/api-client` consumes it today. Freeze `v1` at
  product `1.0`.
- **Stability starts at 1.0, and only for the supported surface** (#591). Before 1.0 nothing is stable,
  and a change needs no deprecation. From 1.0, the operations in `docs/architecture/api-surface.md` are
  retired only after at least **30 days** of `Deprecation` and `Sunset` headers on their responses
  (`backend/src/deprecation.rs`) plus a changelog entry. Everything outside that list stays internal.
- **Monorepo strategy** — one version for the whole deployed unit; backend, web, and docs always
  ship from the same commit under the same `VERSION`+sha tag. Internal, never-published
  crates/packages (`ois-core`, `ois-client`, `discord`, `packages/ui`, `packages/api-client`) stay
  pinned at `0.0.0` — they're workspace-internal and version-irrelevant, not independently
  versioned.

---

## Environment variables

The full list with dev defaults is in `.env.example`. The ones that gate functionality:

- **VATSIM OAuth** (required to sign in): `VATSIM_CLIENT_ID`, `VATSIM_CLIENT_SECRET`,
  `VATSIM_REDIRECT_URI`, `VATSIM_DEV_MODE`.
- **Server admin bootstrap**: `OIS_SERVER_ADMIN_CID` (comma-separated CIDs) — the only way to grant
  `SERVER_ADMIN`.
- **VATUSA** (optional roster sync): `VATUSA_API_BASE`, `VATUSA_API_KEY`, `OIS_PUBLIC_URL`, and
  `OIS_SECRET_KEY` (32 base64 bytes; encrypts the webhook secret — without it there is no webhook, the
  daily division pull still runs). See `docs/features/vatusa-sync.md`.
- **Discord bot** (optional): `DISCORD_BOT_TOKEN`, `OIS_API_BASE`, `OIS_API_TOKEN`, `OIS_POLL_SECS`.
- **Web/Vite dev**: `VITE_OIS_API_URL`, `OIS_OPENAPI_URL` (codegen source) — in `web/.env.local`.
- **Observability** (optional, second compose file): `METRICS_TOKEN` gates `GET /metrics` when set;
  `PROMETHEUS_PORT`, `PROMETHEUS_RETENTION`, `GRAFANA_PORT`, `GRAFANA_ADMIN_PASSWORD`. See
  `docs/deploy.md`.
- **Origins** — two lists, deliberately separate. `CORS_ALLOWED_ORIGINS` grants *credentialed* CORS
  (the web app, and the Tauri webview's `tauri://localhost` / `http://tauri.localhost`) and is also
  accepted for OAuth `return_to`. `OAUTH_RETURN_TO_ORIGINS` is `return_to`-only, no CORS — the
  desktop app's loopback listener (`http://127.0.0.1:8765`) belongs here, because that port is bound
  only while sign-in runs and anything else that binds it must not inherit API access (#346).

Never put secrets in the repo; `.env` / `web/.env.local` are gitignored.
