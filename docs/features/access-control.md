# Access control  *(built)*

The fine-grained permission system and its editor — the backbone every other feature
authorizes against. This was the first domain built; every other spec in this directory
has since shipped too (each carries its own status banner noting how it diverged from
its original design) — see [features/README.md](README.md) for the current index.

## Problem

VATUSA capabilities today are coarse and role-name-implied ("is this person staff?").
Leadership wants finegrained control: exactly who can do exactly what, scoped to a
facility where appropriate, with an audit trail of every change.

## Model

A permission is a path string `segments.action` (e.g. `events.items.create`,
`tmu.tmi.publish`, `ace.requests.claim`). Nothing is implied by role name — every
capability is an explicit grant. Full model in
[../architecture/permissions.md](../architecture/permissions.md).

- **Roles** bundle permissions; **direct grants** add or explicitly **deny** for one
  person. A deny beats any allow.
- **SERVER_ADMIN** (env-bootstrapped, never grantable in the UI) holds everything.
- Grants carry an optional **ARTCC scope** (`artcc_id`, NULL = national).

## Permissions

| Permission | Purpose |
| --- | --- |
| `access.self.read` | read your own effective access |
| `access.catalog.read` | read the assignable roles + permission catalog |
| `access.users.read` | read another user's access |
| `access.users.update` | change another user's roles + grants |

## API  *(implemented)*

| Method + path | Permission | Purpose |
| --- | --- | --- |
| `GET /api/v1/access/catalog` | `access.catalog.read` | assignable roles + permission tree (drives the editor UI) |
| `GET /api/v1/access/self` | `access.self.read` | the caller's own effective access |
| `GET /api/v1/admin/users?q=&page=&page_size=` | `access.users.read` | **paginated all-users browser** → `AdminUserPage` |
| `GET /api/v1/admin/users/{cid}/access` | `access.users.read` | a target's grants + roles, **grouped by scope** (national + per-ARTCC) |
| `POST /api/v1/admin/users/{cid}/access` | `access.users.update` | save grants + roles **per scope**; **requires a reason** |
| `GET /api/v1/facilities` | public | the ARTCC list a grant can be scoped to |

The **all-users browser** (`GET /api/v1/admin/users`) takes an optional `q` (name
substring or CID prefix; empty lists everyone), a 1-based `page` (default 1), and
`page_size` (default 25, clamped 1–100). It returns
`AdminUserPage { items, total, page, page_size }`, where each `AdminUserRow` is
`{ cid, display_name, rating, roles[] }` (the distinct role names the user holds across
any scope, for at-a-glance badges). This backs the editor's browsable table; search is
optional, not required.

Read and save are **scope-aware**: the payload is a list of scopes (`artcc_id = null`
national, or a facility id), each carrying a permission tree and, optionally, a role
set. Each listed scope replaces that scope's grants; scopes not listed are untouched.

The save path enforces:

- a **required, non-empty reason** → written to `access.audit_logs` with before/after
  snapshots (the "recorded on this controller's log" trail);
- **catalog + facility validation** — unknown permissions, roles, or ARTCCs are rejected
  (a scoped `artcc_id` is an FK to `org.facilities`);
- **self-scope guards** — a non-SERVER_ADMIN may only add/remove grants they themselves
  hold, and only assign roles they hold, within the scopes they edit — so no one can
  escalate a target above their own authority.

## UI  *(built)*

The web editor lives in the admin portal at `/admin/access`. It now opens on a
**browsable, paginated table of all users** (`GET /api/v1/admin/users`) with an optional
search box — no longer a CID-only lookup. Pick a controller from the table (or search by
name/CID), pick a scope (National or an ARTCC), toggle roles and grouped/collapsible
permission checkboxes (with search + per-group select-all), enter a required reason, and
save. Editing one scope leaves the others untouched. Verified end-to-end against the
backend.

## Built since the first cut

- **Per-permission scope enforcement** — facility-scoped handlers now check scope
  directly via `access_repo::permission_scope(user, permission).allows(facility_id)`
  (e.g. `flow.facility_map.update`, `events.rate.update`, `events.support.update`,
  `events.config.update`). The `RequirePermission<P>` gate still admits the holder
  regardless of scope, and the global **effective-permissions view** does not yet
  pre-filter grants by scope — so the scoped check is the handler's responsibility today.
- **Audit-log read** — `GET /api/v1/admin/audit` (`handlers/audit.rs`) surfaces the
  trail. Every successful mutation is auto-logged by the audit middleware
  (`backend/src/audit.rs`) to `access.audit_logs`.
- **Service-account management** — `/api/v1/admin/service-accounts` (list/create) plus
  `/{id}/rotate`, `/{id}/disable`, and `/{id}/roles` (`handlers/service_accounts.rs`)
  for bot/service credentials.
- **User API keys** — user-owned personal access tokens whose authority is capped by the
  owner's live permissions on every request, with lifecycle + usage auditing. Own domain;
  see [api-keys.md](api-keys.md).

## Not yet

- **Scope-filtered effective view** — the global effective-permissions computation still
  resolves grants without applying their `artcc_id` scope; scope is only enforced where a
  handler opts into the per-permission check above.
