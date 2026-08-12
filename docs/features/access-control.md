# Access control  *(built)*

The fine-grained permission system and its editor — the backbone every other feature
authorizes against. This is the one domain that is **implemented and working today**
(the rest of the feature specs are forward-looking).

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
| `GET /api/v1/admin/users/{cid}/access` | `access.users.read` | a target's roles + effective permissions |
| `POST /api/v1/admin/users/{cid}/access` | `access.users.update` | save roles + grants; **requires a reason** |

The save path enforces:

- a **required, non-empty reason** → written to `access.audit_logs` with before/after
  snapshots (the "recorded on this controller's log" trail);
- **catalog validation** — unknown permissions or roles are rejected;
- **self-scope guards** — a non-SERVER_ADMIN may only add/remove grants they themselves
  hold, and only assign roles they hold, so no one can escalate a target above their
  own authority.

## UI

The web editor renders the catalog as grouped, collapsible permission checkboxes with
a search box and a reason field — the permission-editor screen this project started
from. *(Backend done; the web UI is Phase 3.)*

## Not yet

- **ARTCC-scoped grants** in the editor (schema supports it; the editor writes national
  grants for now).
- **Audit-log read** endpoint (`GET /admin/audit`) to surface the trail in the UI.
- **Service-account management** endpoints (create/rotate/revoke bot credentials).
