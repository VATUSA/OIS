# Event operations

> **Realigned to the shipped implementation.** The original spec modelled an OIS
> operational record with a positions/slots roster and `events.items.*` /
> `events.positions.*` / `events.slots.*` permissions. What actually shipped is a
> **VATUSA event cache** that anchors per-event planning: DCC coordination, facility
> support levels, per-airport rates, staffing requests, TMI packages, and a
> capture/stats window. The permissions are `events.plan.*`, `events.rate.update`,
> `events.config.update`, `events.support.update`, and `events.staffing_requests.*` —
> not the `events.items`/`positions`/`slots` set in the old spec. This doc describes what
> exists in code.

## Problem

Event **creation, review/approval, and public posting stay in the current VATUSA
website** — OIS does not replace them. What OIS owns is the operational layer *around* an
event: the coordination just before, during, and just after it. OIS syncs the canonical
event from VATUSA into a local cache and hangs the planning modules off it.

## Scope

**Built**

- **VATUSA event cache** — events are synced from VATUSA into `events.event` (id, title,
  body/banner, host facility, start/end, review status). This is a read-through mirror,
  not an authored record.
- **DCC request** — per-event DCC coordination status + notes.
- **Facility support** — per-facility support level (required/…) with notes; managed by
  the facility's own staff (facility-scoped).
- **Airport rates** — per-event, per-airport AAR/ADR for planning; facility-scoped.
- **Staffing requests** — per-facility positions requested/filled with an
  `open/met/closed` status.
- **TMI packages** — named bundles of planned traffic-management items
  (`program`/`restriction`/`ground_stop`) that can be **activated** / **deactivated** as
  a unit, plus per-item CRUD.
- **Capture + stats** — a saved capture window over the event, and computed event stats
  (backed by the `stats` domain).

**Not built (from the original spec)**

- The `events.events` operational record with a `coordination_status` lifecycle.
- The positions roster and controller **slots** sign-up/booking.
- The Discord coordination-thread / staffing-notification outbound jobs.

## Data model

Schema `events` (sqlx migrations `0016`–`0021`, `0037`–`0038`); the capture window lives
in `stats.event_capture` (`0040`). Rows reference `identity.users(id)` for editor
columns; `event_id` is the VATUSA event id (bigint).

### `events.event` — the VATUSA cache *(0016)*

pk `id` (VATUSA event id). `title`, `body` (HTML/BBCode blurb), `banner_image_url`,
`facility` (host ARTCC), `start_time`/`end_time`, `review_status`, `synced_at`.

### `events.dcc_request` *(0017)*

pk `event_id` (cascade). `status` (default `not_needed`), `notes`.

### `events.facility_support` *(0018, scope 0037)*

`(event_id, facility)`. `level` (default `required`), `notes`.

### `events.airport_rate` *(0019)*

`(event_id, icao)`. `aar`/`adr` (0–200), `artcc` (owning ARTCC for the scope check).

### `events.staffing_request` *(0020)*

`(event_id, facility)`. `positions_requested`/`positions_filled` (0–999), `status`
`open`/`met`/`closed`, `notes`.

### `events.tmi_package` + `events.tmi_package_item` *(0021, archive 0038)*

`tmi_package`: `id`, `event_id` (cascade), `name`, `status` `draft`/`activated`,
`activated_at`. `tmi_package_item`: `id`, `package_id` (cascade), `kind`
(`program`/`restriction`/`ground_stop`), `payload` jsonb.

## Permissions

Path-based `segments.action` with `RequirePermission<P>`; several are **facility-scoped**
via a per-permission scope check in the handler (`permission_scope(...).allows(artcc)`).

| permission | purpose | scope |
| --- | --- | --- |
| `events.plan.read` | view an event + all planning modules | read |
| `events.plan.update` | edit DCC, TMI packages (create/delete/activate) | national/staff |
| `events.rate.update` | set an event's airport AAR/ADR | facility-scoped |
| `events.config.update` | manage an airport's default runway configs | facility-scoped |
| `events.support.update` | set a facility's event support level | facility-scoped |
| `events.staffing_requests.read` | view staffing requests | read |
| `events.staffing_requests.create` | create/update/delete staffing requests | staff |
| `events.staffing_requests.decide` | acknowledge/decline a request | staff |

The capture-window write reuses **`stats.capture.update`** (it writes `stats.event_capture`),
and the wind forecast + airport-config reads use `events.plan.read`.

**Not used.** `events.staffing_requests.decide` is seeded but not yet wired to a handler;
the original spec's `events.items.*`, `events.positions.*`, `events.slots.claim`,
`events.discord.publish`, and `events.debrief.*` permissions are **not** implemented.

## API

Versioned REST under `/api/v1`, handlers in `backend/src/handlers/events.rs`
(airport-config + forecast in `handlers/airport_configs.rs`).

| method + path | permission |
| --- | --- |
| `GET /events` | `events.plan.read` |
| `GET /events/{id}` | `events.plan.read` |
| `GET /events/{id}/dcc` | `events.plan.read` |
| `PUT /events/{id}/dcc` | `events.plan.update` |
| `GET /events/{id}/facilities` | `events.plan.read` |
| `PUT / DELETE /events/{id}/facilities/{facility}` | `events.support.update` (facility-scoped) |
| `GET /events/{id}/rates` | `events.plan.read` |
| `PUT / DELETE /events/{id}/rates/{icao}` | `events.rate.update` (facility-scoped) |
| `GET /events/{id}/staffing` | `events.plan.read` |
| `PUT / DELETE /events/{id}/staffing/{facility}` | `events.staffing_requests.create` |
| `GET /events/{id}/packages` | `events.plan.read` |
| `POST /events/{id}/packages` | `events.plan.update` |
| `DELETE /events/{id}/packages/{package_id}` | `events.plan.update` |
| `POST /events/{id}/packages/{package_id}/items` | `events.plan.update` |
| `DELETE /events/{id}/packages/{package_id}/items/{item_id}` | `events.plan.update` |
| `POST /events/{id}/packages/{package_id}/activate` | `events.plan.update` |
| `POST /events/{id}/packages/{package_id}/deactivate` | `events.plan.update` |
| `GET /events/{id}/capture` | `events.plan.read` |
| `PUT /events/{id}/capture` | `stats.capture.update` |
| `GET /events/{id}/stats` | `events.plan.read` |

Reusable per-airport runway configs (used by the event-day planner) live under
`airport-configs`:

| method + path | permission |
| --- | --- |
| `GET /airport-configs/{icao}` | `events.plan.read` |
| `POST /airport-configs/{icao}` | `events.config.update` (facility-scoped) |
| `PUT / DELETE /airport-configs/{icao}/{id}` | `events.config.update` (facility-scoped) |
| `GET /forecast/{icao}` | `events.plan.read` (Open-Meteo wind forecast) |

## Not built

- The operational-record lifecycle, positions roster, and controller slot booking.
- Cross-ARTCC staffing **notifications** and the Discord coordination thread (the
  `integration` outbound-queue plumbing exists, but no `events` handler enqueues today).
- A post-event debrief record.
