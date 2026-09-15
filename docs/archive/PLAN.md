# OIS — Plan of Action

> **Archived — historical.** This is the pre-launch build plan, kept as a record of the decisions
> that shaped the platform. It does not reflect current status; see [`docs/README.md`](../README.md)
> and [`docs/features/`](../features/) for what's actually built today.

## What OIS is

OIS (Event Operational Information System) — one monorepo (backend, web, Discord bot,
and later a desktop app) that brings VATUSA's **event operations, traffic management
(superseding [vatflow.io](https://github.com/djbrombizzle/vatflow)), ACE, and access
tooling** under one platform. It **works alongside the current VATUSA website** — which
retains event creation, review, and posting — rather than replacing it wholesale.
National scale (all ARTCCs), built on the architecture proven in [osmium](../../osmium).

## Locked decisions

| Decision           | Choice                                      | Implication                                                                                                                      |
|--------------------|---------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------|
| Backend origin     | **Port osmium's core, build fresh domains** | osmium's pure permission logic → `crates/ois-core`; DB-backed parts rebuilt in `backend`; domain models are new.                 |
| Permission scoping | **Per-ARTCC scope from day one**            | `access.user_roles` / `access.user_permissions` get a nullable `artcc_id` (`NULL` = national). Enforcement rolls out per-domain. |
| Discord ↔ backend  | **Outbound queue + REST**                   | Backend enqueues `integration.outbound_jobs`; bot drains them and calls back as a service account. Bot has **no** DB access.     |

## Architecture at a glance

- **Backend**: single Axum (Rust 2024) binary over Postgres, per-domain schemas, sqlx with embedded migrations,
  versioned REST under `/api/v1`, self-served OpenAPI. Repo-backed query layer; handlers stay thin. Typed
  `RequirePermission<P>`
  extractors enforce access in the handler signature.
- **Permissions**: explicit path-based (`segments.action`), allow/deny grants, role + direct grants, SERVER_ADMIN
  singleton, per-ARTCC scope. See
  [architecture/permissions.md](architecture/permissions.md).
- **Discord**: backend enqueues jobs; bot executes and reports back via REST.
- **Web**: Vite + React (TanStack Router + Query) consuming a client generated from the backend's OpenAPI; shadcn/ui in the shared `packages/ui`.
- **Desktop**: Tauri, sharing UI packages but with native-only features.

## Phased build order

### Phase 0 — Foundation *(in progress)*

- [x] Monorepo scaffold: Cargo + pnpm/turbo workspaces, `justfile`, compose, CI dirs.
- [x] Port osmium's pure permission core into `crates/ois-core` (`PermissionPath`, action enum, tree build/normalize) +
  draft OIS catalog.
- [x] Backend skeleton: Axum app, state, config, error type, `/health` (DB probe).
- [x] Access schema (`identity`, `access`) **with `artcc_id` scope**; effective-perms view (role ∪ server-admin-all ∪
  grants − denies); `RequirePermission<P>` +
  `ensure_permission` (DB-backed). Validated in SQL: baseline grant, admin cross-join, deny-beats-allow.
- [x] VATSIM OAuth login + callback + sessions; `/me`; logout. Server-admin bootstrap via `OIS_SERVER_ADMIN_CID`;
  baseline self-service seeding on first login.
- [x] Access management API (the permission-editor backend): `GET /access/catalog`,
  `GET /access/self`, `GET|POST /admin/users/{cid}/access`. Required reason →
  `access.audit_logs` with before/after; self-scope guards (non-admin can only grant/revoke what they hold); catalog +
  role validation. Verified end-to-end.
- [x] Facilities (`org.facilities`) seeded with the VATUSA ARTCCs; `artcc_id` is now a
      real FK on the grant tables; public facilities read API; facility list folded
      into the access catalog. Verified: seed, lookup, 404, FK rejects bogus ARTCC.
- [x] ARTCC-scoped grant editing: the access editor reads/writes grants + roles per
      scope (national + per-ARTCC), FK-validated, with untouched scopes preserved and
      the self-scope guard applied per scope. Verified end-to-end. (Scope-aware
      *enforcement* in the effective-permissions view is deferred to the first domain
      that checks scope.)
- [x] Audit-log read endpoint (`GET /admin/audit`, paged + filterable, `audit.logs.read`).
- [x] Service-account management (`/admin/service-accounts`: create/list/rotate/disable/
      set-roles). Hashed bearer tokens shown once; `SERVER_ADMIN` never assignable.
- [x] OpenAPI mount (utoipa) at `/docs/api/v1/openapi.json` — 15 paths, 14 schemas; the
      URL the `@ois/api-client` codegen reads.
- [ ] VATUSA roster sync (identity, org membership) — facilities now exist to sync into.

### Phase 1 — Feature specs

Write [docs/features/](features/) one spec per capability. Each nails the data model, the exact permission entries, the
API surface, and the Discord touchpoints. This is the "detail how each feature works" step.
See [features/README.md](features/README.md).

### Phase 2 — Backend API (domain by domain)

Order: identity/access → **events** (operational coordination: CC-ARTCC staffing, position sign-up/slots, debrief —
posting/review stays in the current VATUSA site) → **tmu** (NTML/ADV → plain-language, TMI publish, delay page) →
**ace** (support requests) → **discord** outbound queue + config → **flow** (traffic management, VATSIM-data-feed
ingestion). OpenAPI emitted throughout.

### Phase 3 — Web (Vite + React)  *(started)*

Vite + React + TanStack Router/Query. shadcn/ui in the shared `packages/ui` (so the
Tauri desktop reuses the same components + theme). Types generated from the backend's
`openapi.json` with `openapi-typescript` into `packages/api-client` (`@ois/api-client`),
consumed via `openapi-fetch` (`createOisClient(baseUrl)`). **Done so far:** app shell
(nav bar, light/dark theme toggle), VATSIM login + `/me` auth, signed-in dashboard, the
**server admin portal** at `/admin` (sidebar + Overview + Audit Log + Service Accounts),
and the **access-control permission editor** (grouped-checkbox tree + roles + ARTCC scope
+ reason → save). Surfaces still to build: roster, events, TMU/NTML dashboards, ACE
queue, flow dashboards.

### Phase 4 — Discord bot

Drain outbound jobs (auto event threads + staff pings, TMI/ADV embeds, ACE request embeds with claim buttons).
Interaction + slash-command handlers call back via REST. VATSIM↔Discord account linking.

### Phase 5 — Desktop (Tauri)

Native-only tools: live flow/traffic monitor, always-on TMU display, native notifications. Not a wrapper of the website.

## Feature requests → where they land

Mapped from the event feature audit (cobalt/webapps):

Event **posting, review, and approval stay in the current VATUSA website**; OIS owns the
operational window (prior / during / post). Rows below are split accordingly.

| Request                                     | Home                                                   |
|---------------------------------------------|--------------------------------------------------------|
| Approval workflow before public posting     | Current VATUSA site (not OIS)                          |
| Min lead time (no posting within 7 days)    | Current VATUSA site (not OIS)                          |
| Auto cross-post VATUSA→myVATSIM             | Current VATUSA site — pending VATSIM API               |
| AECs (not just ECs) can post                | Current VATUSA site (posting); OIS EC / EVENTS_TEAM roles scope coordination |
| Feature other facilities                    | Current VATUSA site (posting)                          |
| Structured event metadata + API             | Current VATUSA site (posting)                          |
| CC an ARTCC + staffing-request notification | **OIS** events `staffing_requests.*`                   |
| Auto T1 staffing per DP003 (FNOs)           | **OIS** events / jobs                                  |
| Event position sign-up / slots              | **OIS** events `positions` / `slots`                   |
| Auto DCC Discord thread + staff ping        | **OIS** events `discord.publish` + discord bot         |
| Post-event debrief                          | **OIS** events `debrief.*`                             |
| NTML/ADV → plain-language TMU section       | **OIS** tmu                                            |
| NTML/ADV onto site + API                    | **OIS** tmu                                            |
| Average delay page                          | **OIS** tmu `delays.read` (via flow data feed)         |
| Traffic-management tooling (vatflow/SimTraffic) | **OIS** flow — native reimplementation, VATSIM data feed |
| ACE requests merged into site               | **OIS** ace                                            |
| Auto-post ACE requests to #aceteam-requests | **OIS** ace + discord                                  |
| Notify ECs on claim/book                    | **OIS** ace + discord                                  |

## Local validation

```bash
just ci   # fmt-check + cargo check + rust tests + pnpm lint/typecheck
```
