# Data model

Postgres, organized into **per-domain schemas** so each area of the platform owns its
tables. Migrations are embedded in the backend binary and applied on startup (sqlx).

## Schemas

| Schema | Owns |
| --- | --- |
| `platform` | shared helpers (`touch_updated_at`), job bookkeeping |
| `identity` | users, sessions |
| `access` | roles, permissions, grants, service accounts, actors, audit log |
| `org` | facilities (ARTCCs), roster/membership *(planned)* |
| `events` | operational coordination: event record, hosts, positions, slots, staffing requests, debrief *(planned; posting/review stays in the current VATUSA site)* |
| `tmu` | NTML entries, advisories, TMIs, delay samples *(planned)* |
| `ace` | support requests, ACE team roster *(planned)* |
| `flow` | flow programs, traffic data *(planned)* |
| `integration` | Discord config, the outbound-job queue *(planned)* |
| `stats` | connection/traffic statistics *(planned)* |
| `media` | files + metadata *(planned)* |
| `web` | site content (broadcasts, pages) *(planned)* |

## Identity

- `identity.users` — one row per person, keyed by VATSIM `cid`. Carries email, names,
  rating, status.
- `identity.sessions` — issued at login; `session_token` (cookie), `expires_at`,
  `revoked_at`.

## Access (the heart of the platform)

Explicit, path-based permissions — nothing is implied by role name. See
[permissions.md](permissions.md) for the model; the tables:

- `access.roles`, `access.permissions` — the catalogs.
- `access.role_permissions` — role → permission.
- `access.user_roles (user_id, role_name, artcc_id?)` — a role grant, optionally
  **scoped to an ARTCC** (`artcc_id` NULL = national).
- `access.user_permissions (user_id, permission_name, granted, artcc_id?)` — a direct
  grant; `granted = false` is an explicit **deny** that beats any allow.
- `access.service_accounts` + `service_account_credentials` (hashed) + `service_account_roles` — machine clients.
- `access.actors` + `access.audit_logs` — who did what, with the required `reason` and
  before/after snapshots (the access editor's "recorded on this controller's log").

Effective permissions are computed by the `v_effective_user_permissions` view:
role-derived ∪ SERVER_ADMIN (all permissions) ∪ direct grants, minus explicit denies.

### The ARTCC scope

`artcc_id` is on the grant tables **from day one** (NULL = national). It lets a person
be, say, an EC scoped to ZDC without being a national EC. The schema carries the scope
now; per-domain enforcement (and a facility-scoped access editor) roll out as each
domain is built. `org.facilities` will be the referenced ARTCC catalog.

## Conventions

- Text UUID primary keys (`gen_random_uuid()::text`).
- `created_at` / `updated_at` timestamptz, with an `updated_at` touch trigger.
- Status columns are `check`-constrained enums (e.g. event `coordination_status`).
- Foreign keys cascade on delete where a child cannot outlive its parent.
