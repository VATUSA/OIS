# Flow (traffic management)

## Problem

VATUSA's traffic-management / flow tooling lives in an external tool (vatflow — the
modern successor to the older SimTraffic; both do fundamentally the same job). Those
capabilities sit outside the platform, don't share identity/permissions/data, and
expose no API to integrate against.

OIS brings this functionality **in-house as native features** in the `flow` domain — we
are not cloning or embedding vatflow/SimTraffic, we are reimplementing the capabilities
they provide as our own handlers, so they share the platform's identity, permissions,
and data and can feed other domains (notably the TMU average-delay page).

## Scope

The `flow` domain is a set of capabilities (separate handlers, not a single app port):

**First cut**
- **Flow programs / initiatives** — define and publish traffic-management programs
  (the core of what vatflow does today).
- **Live traffic view** — current aircraft positions/phases, sourced from the VATSIM
  data feed.
- **Traffic/timing data feed** — ingest the VATSIM data feed and derive the timing
  milestones (gate-out→wheels-up, entry→landing) that power the TMU average-delay page.

**Later**
- Automated advisory suggestions off the traffic data.
- Historical analytics / trends beyond the delay feed.

The precise capability list is drawn from what vatflow provides today (a feature audit
is still pending — see Open questions), but each is implemented natively in OIS.

## Data model  *(draft)*

Domain: `flow`, plus traffic/timing tables the delay feed reads.

### `flow.programs`

| column | type | notes |
| --- | --- | --- |
| `id` | text pk | |
| `name` | text | |
| `params` | jsonb | program parameters (rates, scope, constraints) |
| `status` | text | `draft` → `published` → `ended` |
| `owner_artcc_id` | text | → `org` ARTCC |
| `published_by` / `published_at` | text / timestamptz null | |

### `flow.program_facilities`

Which facilities a program applies to (`program_id`, `artcc_id`).

### Traffic data (ingested from the VATSIM data feed)

A background job polls the public **VATSIM data feed** and stores samples, from which
per-flight timing milestones are derived:

| table | holds |
| --- | --- |
| `flow.traffic_samples` | raw position/phase snapshots per aircraft per poll |
| `flow.flight_events` | derived milestones per flight: gate-out, wheels-up, entry, landing |
| `flow.traffic_rollups` | aggregated timings (feeds the TMU delay page) |

`tmu.delay_samples` (see [tmu-ntml-adv-tmi.md](tmu-ntml-adv-tmi.md)) reads from the
rollups; the flow live view reads recent samples.

## Permissions

| permission | purpose | holders |
| --- | --- | --- |
| `flow.programs.read` | view programs | public / TMU staff |
| `flow.programs.create` | define a program | `NTMO`, ARTCC-scoped TMU staff |
| `flow.programs.update` | edit a program | same |
| `flow.programs.publish` | publish a program | `NTMO`, ARTCC-scoped TMU staff |
| `flow.programs.delete` | remove a program | `NTMO` |
| `flow.data.read` | live traffic view + delay feed | public (reads may be unauthenticated) |

Ingestion is a **backend job**, not a user-facing permission.

## API

Versioned REST under `/api/v1`. Reads may be public; writes gated.

| method + path | permission | purpose |
| --- | --- | --- |
| `GET /api/v1/flow/programs` | `flow.programs.read` | list programs |
| `POST /api/v1/flow/programs` | `flow.programs.create` | define a program |
| `PATCH /api/v1/flow/programs/{id}` | `flow.programs.update` | edit |
| `POST /api/v1/flow/programs/{id}/publish` | `flow.programs.publish` | publish |
| `DELETE /api/v1/flow/programs/{id}` | `flow.programs.delete` | remove |
| `GET /api/v1/flow/traffic` | `flow.data.read` | live positions/phases from the VATSIM feed |

The delay feed derived from the traffic data is consumed by the TMU domain.

## Data ingestion

A background worker under `backend/src/jobs` polls the **VATSIM data feed** on an
interval, writes `flow.traffic_samples`, and derives `flow.flight_events` /
`flow.traffic_rollups`. This is the single traffic/timing source for both the flow live
view and the TMU average-delay page — there is no separate "SimTraffic" build.

## Open questions

- **Vatflow capability audit** — enumerate exactly which of vatflow's features carry
  over and how they map to `flow` handlers, before locking scope.
- **Program ↔ TMI relationship** — is a flow program the same publish as a TMU TMI, or
  linked to one? (Cross-ref [tmu-ntml-adv-tmi.md](tmu-ntml-adv-tmi.md).)
- **Traffic data granularity + retention** — poll cadence, how long raw samples are
  kept vs. rollups, and which milestones are derivable from the VATSIM feed alone.
- **Domain placement** — whether traffic tables live in `flow` or a shared `stats`
  schema.
