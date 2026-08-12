# OIS — Plan of Action

## What OIS is

The VATUSA platform: one monorepo replacing the current cobalt (backend) + webapps
(frontend) + [vatflow.io](https://github.com/djbrombizzle/vatflow) stack, adding first-class Discord integration and,
later, a native desktop app. National scale (all ARTCCs), built on the architecture proven in [osmium](../../osmium).

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
- **Web**: Next.js consuming a client generated from the backend's OpenAPI.
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
- [ ] OpenAPI mount (utoipa) — deferred until more domains exist.
- [ ] VATUSA roster/facility sync (identity, org).
- [ ] Service account management endpoints + API keys (bot auth). *(schema + bearer resolution already in place; needs
  create/rotate endpoints.)*

### Phase 1 — Feature specs

Write [docs/features/](features/) one spec per capability. Each nails the data model, the exact permission entries, the
API surface, and the Discord touchpoints. This is the "detail how each feature works" step.
See [features/README.md](features/README.md).

### Phase 2 — Backend API (domain by domain)

Order: identity/access → **events** (approval workflow, lead-time, featured, CC-ARTCC staffing, slots) → **tmu**
(NTML/ADV → plain-language, TMI publish, delay page) → **ace** (support requests) → **discord** outbound queue +
config → **flow** (vatflow replacement) → sim-traffic. OpenAPI emitted throughout.

### Phase 3 — Web (Next.js)

Types generated from the backend's `openapi.json` with `openapi-typescript` into the shared `packages/api-client`
(`@ois/api-client`), consumed via `openapi-fetch`
(`createClient<paths>`) and per-domain TanStack Query hooks — the same tooling as osmium's website, packaged so
`desktop` reuses it. Surfaces: the permission editor (grouped-checkbox UI), roster, events, TMU/NTML dashboards, ACE
queue, flow dashboards.

### Phase 4 — Discord bot

Drain outbound jobs (auto event threads + staff pings, TMI/ADV embeds, ACE request embeds with claim buttons).
Interaction + slash-command handlers call back via REST. VATSIM↔Discord account linking.

### Phase 5 — Desktop (Tauri)

Native-only tools: live flow/traffic monitor, always-on TMU display, native notifications. Not a wrapper of the website.

## Feature requests → where they land

Mapped from the event feature audit (cobalt/webapps):

| Request                                     | Home in OIS                                            |
|---------------------------------------------|--------------------------------------------------------|
| Approval workflow before public posting     | events (Phase 2) — port osmium `review_status` model   |
| Min lead time (no posting within 7 days)    | events (Phase 2) — `CreateEvent` validation            |
| Auto cross-post VATUSA→myVATSIM             | events — pending VATSIM API                            |
| AECs (not just ECs) can post                | access — `AEC` role, first-class (Phase 0/2)           |
| CC an ARTCC + staffing-request notification | events `staffing_requests.*` (Phase 2)                 |
| Auto T1 staffing per DP003 (FNOs)           | events / jobs (Phase 2)                                |
| Event slots (booking/signup)                | events `positions`/slots (Phase 2)                     |
| Feature other facilities                    | events `featured.*` (Phase 2)                          |
| Structured event metadata + API             | events API (Phase 2) — multi-host, not single facility |
| Auto DCC Discord thread + staff ping        | discord + events `discord.publish` (Phase 4)           |
| NTML/ADV → plain-language TMU section       | tmu (Phase 2)                                          |
| NTML/ADV onto site + API                    | tmu (Phase 2)                                          |
| Average delay page                          | tmu `delays.read` (Phase 2)                            |
| SimTraffic migration                        | flow / sim-traffic — **build our own**, not SimTraffic |
| ACE requests merged into site               | ace (Phase 2)                                          |
| Auto-post ACE requests to #aceteam-requests | ace + discord (Phase 4)                                |
| Notify ECs on claim/book                    | ace + discord (Phase 4)                                |

## Local validation

```bash
just ci   # fmt-check + cargo check + rust tests + pnpm lint/typecheck
```
