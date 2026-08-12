# ACE support requests

## Problem

The ACE Team provides live controller coverage/support; controllers ask for that support today through off-site channels
(Discord DMs, forum posts), and the OIS ACE page is **hardcoded mock data** with no backend wiring. The backend has an
`ace` schema and an `ACE` role but no `requests` concept and no `team` table, so nothing on the page is real.

This feature makes ACE support a first-class OIS workflow:

- Merge ACE Team request handling into the OIS site (one queue, one source of truth).
- Let a controller **request ACE support** from the website.
- Replace the mock ACE display page with real backend data.
- Auto-post each new request to Discord (`#aceteam-requests`) with an interactive **claim** button.
- Notify the requesting ARTCC's **EC** when a request is claimed/booked.

## Scope

### First cut

- `ace.requests` table + repo + thin handlers; status lifecycle `open → claimed → completed/cancelled`.
- Controller submits a request from the site (`POST /api/v1/ace/requests`).
- Request queue on the site backed by `GET /api/v1/ace/requests` (replaces the mock list on the ACE page).
- On create, enqueue an `ace_request_post` outbound job → embed in `#aceteam-requests` with a claim button.
- Claim (site button or Discord button) → `POST /api/v1/ace/requests/{id}/claim`; enqueue `ace_request_notify` to ping
  the requesting ARTCC's EC and edit the embed to show the claimer.
- Decide (complete/cancel) via `POST /api/v1/ace/requests/{id}/decide`.
- `ace.team_members` table + read endpoint so the roster block on the ACE page is real backend data.

### Later

- **Booking/scheduling** of a claimed request (agreed date/time, calendar surface) — see Open questions.
- ACE team **roster management UI** (create/update/remove members) beyond read-only display.
- Discord **slash command** to request ACE support without opening the site (bot → backend, mirrors the claim callback).
- Per-ARTCC / per-rating routing of the request embed (currently a single national channel).

## Data model

New tables in the existing `ace` schema (`backend/migrations/0001_extensions_and_schemas.sql` already runs
`create schema if not exists ace;`). Follow OIS conventions: `text` PK `default gen_random_uuid()::text`, status as
`text` + `CHECK` (no Postgres `CREATE TYPE`), nullable `artcc_id text` (NULL = national), and a
`platform.touch_updated_at()` trigger on `updated_at`.

### `ace.requests`

| column        | type          | notes                                                                          |
| ------------- | ------------- | ------------------------------------------------------------------------------ |
| `id`          | `text` PK     | `default gen_random_uuid()::text`                                              |
| `requested_by`| `text`        | requesting controller (CID / identity ref)                                     |
| `artcc_id`    | `text` null   | ARTCC the request is for; drives which EC is notified. NULL = unspecified      |
| `position`    | `text` null   | position/facility the support is requested for (free text, first cut)          |
| `requested_for`| `timestamptz` null | desired coverage time, if the requester supplies one                     |
| `details`     | `text`        | free-text description of the ask                                               |
| `status`      | `text`        | `check (status in ('open','claimed','completed','cancelled'))`, default `open` |
| `claimed_by`  | `text` null   | ACE member who claimed it (CID / identity ref)                                 |
| `claimed_at`  | `timestamptz` null |                                                                           |
| `decided_by`  | `text` null   | who completed/cancelled it                                                     |
| `decided_at`  | `timestamptz` null |                                                                           |
| `discord_message_id` | `text` null | id of the posted embed, so `ace_request_notify` can edit it in place      |
| `created_at`  | `timestamptz` | `default now()`                                                                |
| `updated_at`  | `timestamptz` | `default now()`, `platform.touch_updated_at()` trigger                         |

**Status lifecycle**

```
open ──claim──▶ claimed ──decide──▶ completed
  │                 │
  └──decide─────────┴──decide──────▶ cancelled
```

`open` is the only claimable state. `completed` and `cancelled` are terminal.

### `ace.team_members`

Replaces the hardcoded roster on the ACE display page.

| column       | type          | notes                                                    |
| ------------ | ------------- | -------------------------------------------------------- |
| `id`         | `text` PK     | `default gen_random_uuid()::text`                        |
| `user_id`    | `text`        | identity ref of the ACE member                           |
| `role`       | `text` null   | display role within the ACE team (e.g. lead/member)      |
| `artcc_id`   | `text` null   | home ARTCC, if the member is presented by facility       |
| `active`     | `boolean`     | `default true` — soft-hide instead of delete             |
| `created_at` | `timestamptz` | `default now()`                                           |
| `updated_at` | `timestamptz` | `default now()`, `platform.touch_updated_at()` trigger   |

## Permissions

Path-based `segments.action` entries already registered in `crates/ois-core/src/catalog.rs` (lines ~91–97); this feature
implements the handlers behind them. `claim` uses the OIS-added `Claim` action (`crates/ois-core/src/permissions.rs`).

| permission            | action   | who holds it                        | purpose                                        |
| --------------------- | -------- | ----------------------------------- | ---------------------------------------------- |
| `ace.requests.read`   | `read`   | `ACE`, ACE team members    | view the request queue                         |
| `ace.requests.create` | `create` | any controller (`USER`)             | open a support request                         |
| `ace.requests.claim`  | `claim`  | `ACE`, ACE team members    | claim an `open` request                        |
| `ace.requests.decide` | `decide` | `ACE`                      | complete / cancel a request                    |
| `ace.team.read`       | `read`   | public/`USER` (roster display)      | read the ACE roster                            |
| `ace.team.update`     | `update` | `ACE`                      | manage the roster (Later)                      |

Notes:

- The **service account** the Discord bot authenticates as must hold `ace.requests.claim` so button clicks can call back
  in on behalf of the linked user (per the locked Discord decision — the bot never touches Postgres).
- **ARTCC scope**: assignments carry a nullable `artcc_id` (NULL = national). Scope is stored but
  `access.v_effective_user_permissions` does not yet enforce it (documented Phase-0 gap); whether claim/decide are
  scope-limited is under Open questions. Any scope narrowing is an explicit data-dependent check in the handler, not the
  coarse `RequirePermission` gate.

## API

Versioned REST under `/api/v1`, flat routes registered in `backend/src/router.rs`. Each handler declares
`RequirePermission<P>` in its signature (the only way to satisfy the gate, so a missing check is a visible gap).

| method + path                          | permission gate       | who                          | notes                                                                 |
| -------------------------------------- | --------------------- | ---------------------------- | --------------------------------------------------------------------- |
| `POST /api/v1/ace/requests`            | `ace.requests.create` | requesting controller        | inserts `status='open'`; enqueues `ace_request_post`                  |
| `GET  /api/v1/ace/requests`            | `ace.requests.read`   | ACE team                     | queue; supports `status` / `artcc_id` filters                         |
| `GET  /api/v1/ace/requests/{id}`       | `ace.requests.read`   | ACE team                     | single request detail                                                 |
| `POST /api/v1/ace/requests/{id}/claim` | `ace.requests.claim`  | ACE member / bot svc account | **data-dependent: request must be `status='open'`** (409 otherwise)   |
| `POST /api/v1/ace/requests/{id}/decide`| `ace.requests.decide` | `ACE`               | body `{ outcome: 'completed' \| 'cancelled' }`; must not be terminal  |
| `GET  /api/v1/ace/team`                | `ace.team.read`       | site (roster display)        | real data for the ACE page (replaces mock)                            |
| `PUT/PATCH /api/v1/ace/team/{id}`      | `ace.team.update`     | `ACE`               | roster management (Later)                                             |

**Claim is doubly gated**: the `RequirePermission<AceRequestsClaim>` extractor authorizes the *caller*, and the handler
then re-reads the row inside the transaction and rejects unless `status='open'` — mirroring OIS's convention that
ownership/state checks live explicitly in the handler, not the permission layer. On success it sets
`status='claimed'`, `claimed_by`, `claimed_at`, and enqueues `ace_request_notify`.

## Discord

Locked architecture: the backend inserts rows into `integration.outbound_jobs` (`job_type` text, JSON `payload`,
`status`, `attempt_count`, `next_attempt_at`); the Rust bot drains pending jobs, performs the Discord action, and acks.
Button clicks call back into the API as a service account. The bot owns no data.

| outbound `job_type` | enqueued when                          | payload (key fields)                                           | bot action                                                                 |
| ------------------- | -------------------------------------- | ------------------------------------------------------------- | -------------------------------------------------------------------------- |
| `ace_request_post`  | request created (`status='open'`)      | `request_id`, `requested_by`, `artcc_id`, `position`, `details` | post embed to `#aceteam-requests` with a **CLAIM** button; on ack, store returned `discord_message_id` back on the request |
| `ace_request_notify`| request claimed                        | `request_id`, `claimed_by`, `artcc_id`, `discord_message_id`  | ping/DM the requesting ARTCC's **EC**; **edit the original embed** to show the claimer and disable the button |

Flow:

1. `POST /ace/requests` → row `open` + enqueue `ace_request_post`.
2. Bot posts embed with claim button; acks with the Discord message id → persisted on `ace.requests.discord_message_id`.
3. User clicks **CLAIM** in Discord → bot calls `POST /ace/requests/{id}/claim` as the service account on behalf of the
   linked user (same endpoint the site button uses).
4. Handler flips `open → claimed` and enqueues `ace_request_notify`.
5. Bot notifies the EC and edits the embed in place.

The EC targeted by `ace_request_notify` is resolved from the request's `artcc_id` (facility-scoped `EC` role
holders for that ARTCC). Channel/role mapping lives in the `integration` config tables (edited via
`discord.config.{read,update}`), per the Discord integration spec.

## Open questions

- **Who may claim** — any `ACE`/ACE team member nationally, or scoped by the request's ARTCC and/or the
  claimer's rating? (ARTCC scope is stored on assignments but not yet enforced by the effective-permissions view, so
  scoped claiming would need an explicit handler check.)
- **Booking / scheduling model** for a claimed request: is `claimed` the end state for v1, or does a claim lead to a
  scheduled coverage slot (date/time, confirmation, calendar surface, reschedule/no-show handling)? This determines
  whether we need an `ace.bookings` table or just the `requested_for`/`claimed_*` columns above.
- **"Booked" vs "claimed" for EC notification** — the requirement says notify ECs on *claim/book*. If booking is a
  separate later step, does the EC get pinged on claim, on book, or both?
- **EC resolution when `artcc_id` is NULL** — if a request isn't tied to an ARTCC, who (if anyone) is notified?
- **Request de-duplication / rate limiting** — should a controller be blocked from opening multiple concurrent `open`
  requests?
