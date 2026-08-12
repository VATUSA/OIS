# Event operations

## Problem

Event **creation, review/approval, and public posting stay in the current VATUSA
website** — OIS does not replace them. What's missing today is the operational layer
*around* an event: the coordination just before it, during it, and just after. That
window — pre-event planning, staffing coordination, controller sign-up, live Discord
coordination, and post-event debrief — is what OIS owns.

OIS references an event that already exists (posted and approved) in the current VATUSA
site and provides the coordination surface for it.

## Scope boundary

| In OIS (operational: prior / during / post) | In the current VATUSA website (not OIS) |
| --- | --- |
| Coordination/ops plan for an event | Creating and editing the event posting |
| Position roster + controller sign-up (slots) | Review/approval workflow before public |
| Cross-ARTCC staffing requests (CC an ARTCC) + notifications | Minimum-lead-time enforcement |
| Auto T1 staffing for FNOs (DP003) | Public event listing + featured facilities |
| Discord coordination thread + staff ping | myVATSIM cross-posting |
| Post-event debrief | Structured public event metadata / API |

## Scope

**First cut**
- **Reference the canonical event** — link/sync the event (id, title, start/end,
  hosting ARTCC(s)) from the current VATUSA site into a lightweight OIS operational
  record.
- **Pre-event coordination** — a coordination/ops plan, a position roster, and
  controller **sign-up (slots)**.
- **Cross-ARTCC staffing** — CC an ARTCC on the operation, which fans out a
  staffing-request notification to that ARTCC.
- **During-event** — auto-create a Discord **coordination thread** and ping required
  staff when the operation goes active.
- **Post-event** — a **debrief** record (notes, issues, follow-ups).

**Later**
- Auto-trigger T1 staffing requests for FNOs per policy DP003.
- Scheduling/booking niceties over slots.

## Data model  *(draft)*

Postgres `events` schema. The OIS record is **operational**, keyed to the canonical
event in the current VATUSA site — it does not carry a posting/approval lifecycle.

### `events.events` (operational record)

| column | type | notes |
| --- | --- | --- |
| `id` | text pk | OIS id |
| `source` | text | origin system, e.g. `vatusa_web` |
| `source_ref` | text | id of the canonical event in the current VATUSA site |
| `title` | text | mirrored for display |
| `starts_at` / `ends_at` | timestamptz | mirrored |
| `coordination_status` | text | `planning` → `active` → `complete` (operational, **not** review_status) |
| `synced_at` | timestamptz null | last sync from the source |

Unique `(source, source_ref)`.

### `events.event_hosts`

Hosting / participating ARTCCs — drives coordination and ARTCC scope
(`event_id`, `artcc_id`, `role` = `host` \| `participating`).

### `events.positions`

Position roster for the operation (`event_id`, `callsign`, `user_id?`, `status`
`OPEN`→`REQUESTED`→`ASSIGNED`, `published`). Unique `(event_id, callsign)`.

### `events.slots`

Controller sign-up/booking over positions (`id`, `position_id`, `booked_by?`,
`status` `open`→`requested`→`booked`→`cancelled`).

### `events.staffing_requests`

CC-an-ARTCC and (Later) DP003 T1 auto-trigger (`id`, `event_id`,
`requested_artcc_id`, `origin` `cc`\|`t1_auto`, `status`
`pending`→`acknowledged`/`declined`, `notification_state` `queued`→`sent`/`failed`).

### `events.debrief`

Post-event notes (`id`, `event_id`, `author_id`, `body`, `created_at`).

## Permissions

Path-based `segments.action`, `RequirePermission<P>` + data-dependent ARTCC-scope
checks. Facility grants carry a nullable `artcc_id` (NULL = national). **Nothing here
grants event *posting/approval* — that lives in the current VATUSA site.**

| permission | purpose | holders |
| --- | --- | --- |
| `events.items.read` | view the operational record | scoped staff |
| `events.items.create` | attach/link an OIS operational record to a canonical event | `EC` (ARTCC-scoped); `EVENTS_TEAM` |
| `events.items.update` | edit coordination fields (plan, status) | same |
| `events.items.delete` | remove the operational record | `EC`; `EVENTS_TEAM` |
| `events.positions.assign` | assign a controller to a position | `EC`, `EVENTS_TEAM` |
| `events.positions.publish` | publish the position roster | `EC`, `EVENTS_TEAM` |
| `events.positions.delete` | remove a position | `EC`, `EVENTS_TEAM` |
| `events.positions.self.request` | request a position for yourself | any authenticated controller |
| `events.slots.claim` | book an open slot | any authenticated controller |
| `events.staffing_requests.create` | CC an ARTCC | `EC`; `EVENTS_TEAM` |
| `events.staffing_requests.read` | see incoming requests for your ARTCC | same |
| `events.staffing_requests.decide` | acknowledge/decline a request | `EC` |
| `events.discord.publish` | open the coordination thread + ping staff | `EC`, `EVENTS_TEAM` |
| `events.debrief.read` | read the debrief | scoped staff |
| `events.debrief.create` | write a debrief entry | `EC`, `EVENTS_TEAM` |

## API

Versioned REST at `/api/v1`. The OIS record references the canonical event; how it is
linked/synced from the current VATUSA site is an open question (import job vs. webhook
vs. manual link).

| method + path | purpose | who |
| --- | --- | --- |
| `GET /api/v1/events/{id}` | operational record + hosts + positions | scoped staff |
| `POST /api/v1/events` | attach an operational record to a canonical event | `events.items.create` |
| `PATCH /api/v1/events/{id}` | edit coordination fields | `events.items.update` |
| `POST /api/v1/events/{id}/activate` | mark active (+ open Discord thread) | `events.discord.publish` |
| `GET /api/v1/events/{id}/positions` | roster | scoped staff / controllers |
| `POST /api/v1/events/{id}/positions` | request a position | `events.positions.self.request` |
| `POST /api/v1/events/{id}/positions/{pid}/assign` | assign | `events.positions.assign` |
| `POST /api/v1/events/{id}/slots/{sid}/claim` | book a slot | `events.slots.claim` |
| `POST /api/v1/events/{id}/staffing-requests` | CC an ARTCC | `events.staffing_requests.create` |
| `GET /api/v1/events/staffing-requests?artcc=` | incoming for an ARTCC | `events.staffing_requests.read` |
| `POST /api/v1/events/staffing-requests/{id}/decide` | acknowledge/decline | `events.staffing_requests.decide` |
| `POST /api/v1/events/{id}/debrief` | add a debrief entry | `events.debrief.create` |

## Discord

Follows the locked outbound pattern ([discord-integration.md](discord-integration.md)):
the backend enqueues `integration.outbound_jobs`; the bot performs the action and calls
back as a service account.

- On **activate**, enqueue `event_thread_create` to open a coordination thread/forum
  post and ping the required staff roles.
- A **CC-an-ARTCC** staffing request enqueues a notification to that ARTCC's configured
  channel/role (`notification_state` tracks queued→sent/failed).
- DP003 T1 auto-triggers *(Later)* create `staffing_requests` with `origin = t1_auto`
  and fan out the same way.

## Open questions

- **Event linkage** — how OIS references the canonical event in the current VATUSA
  site: a pull/import job, a webhook from the current backend (cobalt), a shared id, or
  a manual link by staff? Does OIS ever write back?
- **Positions ownership** — does OIS own the position roster, or read it from the
  current site if that site already models positions?
- **DP003 T1 criteria** — exact FNO criteria, timing relative to the event, and which
  ARTCCs/roles get notified.
- **Debrief structure** — free-form notes vs. structured fields (issues, metrics,
  follow-ups) and who can read them.
- **CC-an-ARTCC** — always creates a trackable `staffing_request`, or a
  notification-only CC in some cases?
