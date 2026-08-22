# TMU — traffic management

> **Realigned to the shipped implementation.** The original spec described an
> NTML / advisory (ADV) / TMI tool with a plain-language parser and a public API. What
> actually shipped in the `tmu` domain is a **vatflow-style metering toolset**: TMIs
> (reshaped to the NTML restriction row), airport **rate programs**, **ground stops**,
> **issued CFRs**, and **Ground Delay Programs (GDP)** — plus the live-feed flow / taxi /
> departures views those drive. The NTML-entry and advisory tables and their
> `tmu.ntml.*` / `tmu.adv.*` permissions were **never implemented** (the perms are seeded
> in the catalog but no handler references them). This doc describes what exists in code.
> The GDP feature (create/publish/freeze, board, compress, lock/unlock, revise/extend) is
> feature-complete; this is the domain-level spec.

## Problem

VATUSA's traffic-management tooling (restrictions, rate programs, ground stops, ground
delay programs) lives in an external tool with no shared identity, permissions, or API.
OIS brings it in-house in the `tmu` domain, sharing the platform's auth and the same
ingested VATSIM feed / nav / winds model the `flow` domain uses.

## Scope

**Built**

- **TMIs** — logged restrictions in the NTML row shape (requesting ↔ providing facility,
  a restriction string, a start/stop window) with a `draft → published → expired /
  cancelled` lifecycle.
- **Rate programs** — one per airport: AAR + spacing (minutes/miles-in-trail), per-gate
  restrictions, aircraft exclusions. Live operational config (no publish lifecycle).
- **Ground stops** — hold ground departures into a field, scoped to ARTCC/FIR(s).
- **Issued CFRs** — controller-locked wheels-up (EDCT) for a ground departure into a
  metered field, so a release stops drifting as demand recomputes.
- **Ground Delay Programs (GDP)** — meter inbound demand to an arrival airport down to
  its AAR via Ration-By-Schedule, freezing control times (CTA) + EDCTs at publish.
- **Live views** — per-airport metered arrival flow, taxi stats, and the departure-field
  CFR view, all computed live off the feed.

**Not built (from the original spec)**

- NTML **entry** CRUD and advisories (ADV) as separate authored/parsed records.
- Plain-language parse-on-write rendering.
- A public/versioned read API for advisories/TMIs (only the public "board"
  `GET /api/v1/public/board` and per-flight `GET /api/v1/public/flight/{callsign}`
  advisory exist).
- The average-delay page (`tmu.delays.read` is seeded but unused).

## Data model

Schema `tmu` (sqlx migrations `0008`–`0015`, `0026`–`0027`, `0030`–`0032`). All tables
carry `created_at`/`updated_at`; author/publisher columns reference `identity.users(id)`.

### `tmu.tmis` — Traffic Management Initiatives *(0008, reshaped 0009)*

Migration 0009 dropped the original `kind`/`element`/`reason`/`artcc_id` columns for the
NTML row controllers actually log:

| column | notes |
| --- | --- |
| `id` | pk |
| `requesting` / `providing` | requesting ↔ providing facility |
| `restriction` | the restriction text |
| `start_time` / `stop_time` | active window (renamed from `effective_start/end`) |
| `status` | `draft` → `published` → `expired` \| `cancelled` |
| `published_by` / `published_at` | set on publish |

### `tmu.programs` — airport rate programs *(0010)*

pk `icao`. `aar` (1–200), airport-wide spacing (`trail` minutes-in-trail, or `mit`
miles-in-trail when > 0), `gates` jsonb (`[{name, trail, mit}]`, ≤ 10), aircraft
exclusions (`exclude_wake`, `exclude_types`, `jets_only`). Edited in place — no
draft/publish lifecycle.

### `tmu.ground_stops` — ground stops *(0011, status 0012)*

`airport`, `scope` (space-separated ARTCC/FIR codes; `''` = field-wide), `until` (HHMM
Zulu; null = until further notice).

### `tmu.issued_cfrs` — issued CFRs *(0013)*

pk `callsign` (one active CFR per flight): `airport` (metered field), `wheels_up`
(locked release time), `issued_by`/`issued_at`.

### `tmu.gdp` + `tmu.gdp_slot` — Ground Delay Programs *(0026, scope 0027)*

`tmu.gdp`: `airport`, `aar`, HHMM `start_time`/`end_time`, `max_enroute_min` (scope
tier), `exempt_airborne`, `scope` (departure ARTCCs; `''` = all), `status`
`draft → published → expired/cancelled`. `tmu.gdp_slot` (pk `gdp_id, callsign`, cascade):
frozen `original_eta`, `cta`, `edct`, `delay_min` assigned by Ration-By-Schedule at
publish.

## Permissions

Path-based `segments.action`, enforced with `RequirePermission<P>`; per-ARTCC scope via
nullable `artcc_id` on the grant. The permissions the handlers **actually gate on** (each
seeded by the migration that adds its table):

| permission | action | gated handler(s) |
| --- | --- | --- |
| `tmu.tmi.read` | read | list TMIs |
| `tmu.tmi.create` | create | create TMI |
| `tmu.tmi.update` | update | edit TMI |
| `tmu.tmi.publish` | publish | publish / cancel TMI |
| `tmu.tmi.delete` | delete | delete TMI |
| `tmu.program.read` | read | list programs, flow / taxi / departures views |
| `tmu.program.update` | update | upsert a rate program |
| `tmu.program.delete` | delete | delete a rate program |
| `tmu.groundstop.read` | read | list ground stops |
| `tmu.groundstop.create` | create | issue a ground stop |
| `tmu.groundstop.publish` | publish | publish / cancel a ground stop |
| `tmu.groundstop.delete` | delete | delete a ground stop |
| `tmu.cfr.assign` | assign | issue / release a CFR |
| `tmu.gdp.read` | read | list GDPs, GDP board |
| `tmu.gdp.create` | create | create / revise a GDP |
| `tmu.gdp.publish` | publish | publish / cancel / compress / lock / unlock |
| `tmu.gdp.delete` | delete | delete a GDP |

**Unused catalog entries.** `tmu.ntml.{read,create,update,delete}`, `tmu.adv.*`, and
`tmu.delays.read` are seeded (migration 0008 and `catalog.rs`) from the original spec but
**no handler references them** — they correspond to the NTML/ADV/delay features that were
not built.

## API

Versioned REST under `/api/v1`. TMI / program / ground-stop handlers in
`backend/src/handlers/tmu.rs`; GDP in `handlers/gdp.rs`; the live-feed views and CFRs in
`handlers/feed.rs`.

### TMIs

| method + path | permission |
| --- | --- |
| `GET /tmu/tmis` | `tmu.tmi.read` |
| `POST /tmu/tmis` | `tmu.tmi.create` |
| `PATCH /tmu/tmis/{id}` | `tmu.tmi.update` |
| `DELETE /tmu/tmis/{id}` | `tmu.tmi.delete` |
| `POST /tmu/tmis/{id}/publish` | `tmu.tmi.publish` |
| `POST /tmu/tmis/{id}/cancel` | `tmu.tmi.publish` |

### Rate programs, ground stops, CFRs, live views

| method + path | permission |
| --- | --- |
| `GET /tmu/programs` | `tmu.program.read` |
| `PUT /tmu/programs/{icao}` | `tmu.program.update` |
| `DELETE /tmu/programs/{icao}` | `tmu.program.delete` |
| `GET /tmu/ground-stops` | `tmu.groundstop.read` |
| `POST /tmu/ground-stops` | `tmu.groundstop.create` |
| `POST /tmu/ground-stops/{id}/publish` | `tmu.groundstop.publish` |
| `POST /tmu/ground-stops/{id}/cancel` | `tmu.groundstop.publish` |
| `DELETE /tmu/ground-stops/{id}` | `tmu.groundstop.delete` |
| `GET /tmu/flow/{icao}` | `tmu.program.read` |
| `GET /tmu/departures/{dep}` | `tmu.program.read` |
| `GET /tmu/taxi/{icao}` | `tmu.program.read` |
| `POST /tmu/cfr` | `tmu.cfr.assign` |
| `DELETE /tmu/cfr/{callsign}` | `tmu.cfr.assign` |

### Ground Delay Programs

| method + path | permission |
| --- | --- |
| `GET /tmu/gdp` | `tmu.gdp.read` |
| `POST /tmu/gdp` | `tmu.gdp.create` |
| `PUT /tmu/gdp/{id}` | `tmu.gdp.create` (revise) |
| `DELETE /tmu/gdp/{id}` | `tmu.gdp.delete` |
| `GET /tmu/gdp/{id}/board` | `tmu.gdp.read` |
| `POST /tmu/gdp/{id}/publish` | `tmu.gdp.publish` |
| `POST /tmu/gdp/{id}/cancel` | `tmu.gdp.publish` |
| `POST /tmu/gdp/{id}/compress` | `tmu.gdp.publish` |
| `POST /tmu/gdp/{id}/slots/{callsign}` | `tmu.gdp.publish` (lock) |
| `DELETE /tmu/gdp/{id}/slots/{callsign}` | `tmu.gdp.publish` (unlock) |

The departures view (`/tmu/departures/{dep}`) also surfaces **FCA releases** from the
`flow` domain as frozen CFRs — see [flow.md](flow.md#fca-releases-in-the-departures-view).
Mutations publish the `tmu`/`flow.cfr` realtime topics so open boards nudge-and-refetch.

## Not built

- NTML entries and advisories as first-class records, the plain-language parser, and the
  public advisory/TMI API from the original spec.
- The average-delay page (`tmu.delays.read`).
- Discord embeds on publish (the outbound-queue plumbing is the `integration` domain's,
  and no `tmu` handler enqueues today).
