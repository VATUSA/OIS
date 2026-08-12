# TMU — NTML / ADV / TMI

## Problem

The National Traffic Management Log (NTML), advisories (ADV), and Traffic Management Initiatives (TMI) live on a legacy
tool today, reachable from OIS only as an external link. Staff manage them off-site, consumers scrape or eyeball them,
and there is no API. This feature brings NTML/ADV/TMI fully onto the OIS site with a versioned REST API:

- **NTML/ADV** are authored and managed in OIS (no more legacy tool), then parsed into a plain-language TMU section that
  is readable by non-specialists.
- **TMU staff generate and publish TMIs** through the site.
- Published TMIs and ADVs are exposed via the public API so downstream tools (client add-ons, dashboards, other virtual
  orgs) can consume them.
- An **average-delay page** shows gate-out→wheels-up and entry→landing timings with color-coded thresholds, filterable
  by facility/airport. It depends on the traffic/timing data feed owned by [flow.md](flow.md) (ingested from the VATSIM
  data feed) — treated here as a dependency, not built in this feature.
- On TMI/ADV publish, post a Discord embed to the configured channel.

## Scope

### First cut

- NTML entry CRUD, with parse-on-write into a plain-language rendering.
- Advisory (ADV) CRUD with an effective window and a `draft → published → expired` lifecycle.
- TMI authoring (`draft`) and **publish** by TMU staff, with plain-language rendering.
- Public read endpoints for the plain-language TMU section, published ADVs, and published TMIs (site + external API
  consumers).
- Gated create/update/publish/delete for staff.
- Discord embed on TMI publish and ADV publish.

### Later

- **Average-delay page**: gate-out→wheels-up and entry→landing timings, color threshold bands, filterable by
  facility/airport/time window. Depends on the traffic data feed (see [flow.md](flow.md), ingested from the VATSIM data
  feed); `tmu.delay_samples` below is the read model this feature would query, populated by that feed's ingestion job.
- Linking a TMI to a flow program (`flow.programs`) if the "one publish vs. linked" question in flow.md resolves to
  linked.
- Amendment/supersede chains for ADVs and TMIs (revision history beyond a simple status flip).

## Data model

Schema `tmu` (per-domain schema, sqlx migrations). All tables carry `created_at`/`updated_at`; author/publisher columns
reference the platform user id. `artcc_id` is nullable — null means a national-scope row.

### `tmu.ntml_entries`

| Column | Notes |
| --- | --- |
| `id` | PK |
| `artcc_id` | nullable; owning ARTCC, null = national |
| `element` | affected element (airport / fix / sector / airway), free-form for first cut |
| `raw_payload` | JSON of the source fields as authored/imported (the NTML "row") |
| `plain_language` | rendered plain-language text (parse-on-write output) |
| `event_time` | when the logged event applies |
| `created_by` / `updated_by` | authoring staff |

No status enum — NTML entries are log records (create/edit/delete), not a publish workflow.

### `tmu.advisories`

| Column | Notes |
| --- | --- |
| `id` | PK |
| `artcc_id` | nullable scope |
| `adv_type` | advisory category (e.g. GDP, GS, reroute, information) — enum TBD, see Open questions |
| `title` | short headline |
| `body` | full advisory text |
| `plain_language` | rendered plain-language summary |
| `effective_start` / `effective_end` | effective window (nullable end = until cancelled) |
| `status` | `draft → published → expired` (also `cancelled`) |
| `published_by` / `published_at` | set on publish |

### `tmu.tmis`

| Column | Notes |
| --- | --- |
| `id` | PK |
| `artcc_id` | nullable scope |
| `kind` | TMI kind (e.g. MIT, MINIT, ground stop, ground delay, reroute) — enum TBD |
| `params` | JSON of the initiative parameters (rate, distance, altitude, scope, etc.) |
| `plain_language` | rendered plain-language description |
| `effective_start` / `effective_end` | active window |
| `status` | `draft → published → expired` (also `cancelled`) |
| `published_by` / `published_at` | set on publish |

### `tmu.delay_samples` *(Later — read model for the average-delay page)*

| Column | Notes |
| --- | --- |
| `id` | PK |
| `airport` | ICAO |
| `artcc_id` | nullable facility scope |
| `phase` | which timing this row measures: `gate_to_wheels` or `entry_to_landing` |
| `sample_time` | when the movement occurred |
| `duration_seconds` | measured elapsed time for the phase |

Populated by the traffic-feed ingestion job (a backend job under `backend/src/jobs`, per [flow.md](flow.md)), **not** by
user writes. The average-delay page aggregates these into rolling averages per airport/phase and applies color bands.

## Permissions

Path-based `segments.action`, enforced with `RequirePermission<P>`. Per-ARTCC scope via nullable `artcc_id`; a national
grant covers all ARTCCs. Held by `TMU_NATIONAL` (national scope) and ARTCC-scoped TMU staff (scoped to their ARTCC).

| Permission | Action | Notes |
| --- | --- | --- |
| `tmu.ntml.read` | read | **public** — powers the plain-language section and external consumers |
| `tmu.ntml.create` | create | staff |
| `tmu.ntml.update` | update | staff |
| `tmu.ntml.delete` | delete | staff |
| `tmu.adv.read` | read | **public** — published advisories only for unauthenticated callers |
| `tmu.adv.create` | create | staff |
| `tmu.adv.update` | update | staff |
| `tmu.adv.publish` | publish | staff; flips `draft → published`, triggers Discord |
| `tmu.tmi.read` | read | **public** — published TMIs only for unauthenticated callers |
| `tmu.tmi.create` | create | staff author a draft |
| `tmu.tmi.update` | update | staff edit a draft |
| `tmu.tmi.publish` | publish | staff; flips `draft → published`, triggers Discord (matches `tmu.tmi.publish` referenced in discord-integration.md) |
| `tmu.tmi.delete` | delete | staff |
| `tmu.delays.read` | read | **public** *(Later)* — average-delay page and API |

Reads marked public are anonymous-accessible; for ADV and TMI, anonymous/public reads return only `published` rows,
while staff with the relevant `read` permission also see drafts. Every create/update/publish/delete is gated.

## API

Versioned under `/api/v1`, thin handlers over the repo layer. Read endpoints are public where marked (site + external
API consumers); create/update/publish/delete require the matching permission above. Publish endpoints also enforce a
data-dependent precondition (row must be in `draft`) on top of the permission gate.

| Method + path | Who | Notes |
| --- | --- | --- |
| `GET /api/v1/tmu/ntml` | public | list NTML entries; supports `artcc_id`, time-window filters |
| `POST /api/v1/tmu/ntml` | `tmu.ntml.create` | create entry; parse-on-write fills `plain_language` |
| `PATCH /api/v1/tmu/ntml/{id}` | `tmu.ntml.update` | edit; re-runs parse |
| `DELETE /api/v1/tmu/ntml/{id}` | `tmu.ntml.delete` | remove entry |
| `GET /api/v1/tmu/advisories` | public | published only for anonymous; `?status=` for staff |
| `POST /api/v1/tmu/advisories` | `tmu.adv.create` | create draft |
| `PATCH /api/v1/tmu/advisories/{id}` | `tmu.adv.update` | edit draft/details |
| `POST /api/v1/tmu/advisories/{id}/publish` | `tmu.adv.publish` | `draft → published`; enqueues `adv_publish` job |
| `GET /api/v1/tmu/tmis` | public | published only for anonymous; `?status=` for staff |
| `POST /api/v1/tmu/tmis` | `tmu.tmi.create` | create draft |
| `PATCH /api/v1/tmu/tmis/{id}` | `tmu.tmi.update` | edit draft |
| `POST /api/v1/tmu/tmis/{id}/publish` | `tmu.tmi.publish` | `draft → published`; enqueues `tmi_publish` job |
| `DELETE /api/v1/tmu/tmis/{id}` | `tmu.tmi.delete` | remove TMI |
| `GET /api/v1/tmu/delays` | public *(Later)* | aggregated averages; filters `airport`, `artcc_id`, `phase`, window |

**External exposure**: the public `GET` endpoints for advisories and TMIs are the stable, versioned surface downstream
tools consume. They return the structured record plus the `plain_language` rendering so consumers can display either the
parsed data or the human summary without re-implementing the parse.

## Discord

Locked outbound-queue architecture (see [discord-integration.md](discord-integration.md)): the backend inserts a row
into `integration.outbound_jobs`; the Rust bot drains it, posts the embed, and acks back as a service account. The bot
never touches Postgres.

- **TMI publish** → enqueue `tmi_publish` with a payload of `{ tmi_id, kind, plain_language, effective window, artcc }`.
  Bot posts an embed to the configured TMU channel.
- **ADV publish** → enqueue `adv_publish` with `{ advisory_id, adv_type, title, plain_language, effective window,
  artcc }`. Bot posts an embed to the configured channel.

Channel/role mapping lives in `integration` config, edited via `discord.config.{read,update}` (not owned by this
feature). Only the `published` transition enqueues a job; edits and deletes do not (see Open questions on
edit/cancel notifications).

## Open questions

- **NTML source formats to parse**: what exact fields/formats the legacy NTML rows use (and whether we import history or
  start fresh), so `raw_payload` and the parser can be bounded.
- **Plain-language rules**: the mapping from raw NTML/ADV/TMI parameters to plain-language text — templated per
  `kind`/`adv_type`, or a single generic renderer? Where the rules live (code vs. data) and who maintains them.
- **Enum inventories**: the concrete `adv_type` and TMI `kind` value sets, and whether `params` is validated per kind.
- **Delay thresholds / color bands**: the numeric cutoffs for green/yellow/red on each phase (gate-out→wheels-up and
  entry→landing), and whether they are per-airport or global.
- **Timing ingestion cadence**: the gate-out / wheels-up / entry / landing timestamps are derived from the **VATSIM data
  feed** by [flow.md](flow.md); open here is only the poll cadence/derivation detail. This feature just reads
  `tmu.delay_samples`.
- **Edit/cancel Discord behavior**: should editing or cancelling a published TMI/ADV edit or delete the existing embed
  (as ACE does on claim), or post nothing?
- **TMI ↔ flow program relationship**: whether a TMI is authored independently or generated from a `flow.programs`
  publish (open in flow.md).
