# Data model

Postgres, organized into **per-domain schemas** so each area of the platform owns its
tables. Migrations are embedded in the backend binary and applied on startup (sqlx).

## Schemas

| Schema | Owns |
| --- | --- |
| `platform` | shared helpers (`touch_updated_at`), job bookkeeping |
| `identity` | users, sessions |
| `access` | roles, permissions, grants, service accounts, actors, audit log |
| `org` | facilities (ARTCCs) *(built)*; broader roster/membership beyond what VATUSA already provides *(planned)* |
| `events` | operational coordination: event cache, DCC, facility support, airport rates, staffing requests, TMI packages, debrief *(built; posting/review stays in the current VATUSA site — see [events-workflow.md](../features/events-workflow.md))* |
| `tmu` | NTML entries, TMIs, ground stops, rate programs, GDPs *(built)* |
| `ace` | support requests *(built)* — there is no local ACE team roster; the team is sourced from VATUSA and the per-facility EC comes from the access-control role, not a stored list (migration `0051`) |
| `flow` | FCAs, routes, runway configs, facility-map config, airport surface data, aircraft profiles, traffic data *(built)* |
| `integration` | the outbound-job queue driving the Discord bot, plus Discord guild/channel/role config (`discord_configs`, `discord_channels`, `discord_roles`, `discord_categories`) *(built)* |
| `stats` | connection/traffic statistics, flights, positions, captures *(built)* |
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
- `access.api_keys` + `access.api_key_permissions` (migration 0045) — user-owned personal
  access tokens (`ois_pat_…`, SHA-256-hashed). `api_key_permissions (permission_name, artcc_id?)`
  is the key's granted subset; a key's effective authority is that ∩ the owner's live access,
  computed per request (see [api-keys.md](../features/api-keys.md)). `owner_user_id` cascades, so
  deleting a user drops their keys.
- `access.actors` + `access.audit_logs` — who did what, with the required `reason` and
  before/after snapshots (the access editor's "recorded on this controller's log"). `actors`
  now includes an `api_key` actor type, so a mutation made via a key is attributed to it.

Effective permissions are computed by the `v_effective_user_permissions` view:
role-derived ∪ SERVER_ADMIN (all permissions) ∪ direct grants, minus explicit denies.

### The ARTCC scope

`artcc_id` is on the grant tables **from day one** (NULL = national). It lets a person
be, say, an EC scoped to ZDC without being a national EC. The schema carries the scope
now; per-domain enforcement (and a facility-scoped access editor) roll out as each
domain is built. `org.facilities` will be the referenced ARTCC catalog.

## Operational domains

The built domains now carry real tables (see the migrations for full columns):

- **`tmu`** — TMIs, ground stops, rate programs, and GDPs (with scope + AAR steps).
- **`flow`** — `flow.fca`, `flow.route`, `flow.runway_config`, `flow.runway_saved_config`,
  plus the tables below.
- **`stats`** — member, flight, position, controller_session, snapshot, capture, winds,
  plus `stats.flight_plan` below.
- **`events`** — event record, DCC, facility support, airport rates, staffing, TMI packages.

Tables worth calling out:

- `flow.fca_release` (migration 0023) — frozen CFR releases per FCA: the metered crossing
  time (`cta_ms`) and release/wheels-up time (`edct_ms`) are pinned when a controller issues
  a release, and the metering engine treats a released aircraft as a fixed constraint. Manual
  drag-to-reorder sequence lives on `flow.fca` (`manual_order` text[], `manual_seq` bool).
- `flow.facility_map_config` (migration 0043) — per-ARTCC color-rule config for the public
  facility map: `facility_id` PK (the owning ARTCC), `rules jsonb` (ordered color rules),
  `default_color`, `updated_by` / `updated_at`.
- `stats.flight_plan` (migration 0044) — flight-plan revision history for temporally-faithful
  replay. `(session_id, effective_from)` PK, one row per distinct plan revision. `stats.flight`
  keeps only the latest plan (overwritten each tick), so this table lets replay show the plan
  in force at each instant instead of retroactively rewriting the whole track.

## Conventions

- Text UUID primary keys (`gen_random_uuid()::text`).
- `created_at` / `updated_at` timestamptz, with an `updated_at` touch trigger.
- Status columns are `check`-constrained enums (e.g. event `coordination_status`).
- Foreign keys cascade on delete where a child cannot outlive its parent.
