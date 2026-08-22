# Flow (traffic management)

> **Realigned to the shipped implementation.** The original spec described a
> `flow.programs`-centric design that was never built. The `flow` domain as shipped is
> **FCA / metering-centric** (ported from vatflow): controller-drawn Flow Constrained
> Areas, a live-feed traffic view, shared reference routes, the runway balancer, an
> IDST departure-release console, and the per-facility public map. This doc describes
> what exists in code. Rate programs, ground stops, GDPs, and CFRs live in the `tmu`
> domain — see [tmu-ntml-adv-tmi.md](tmu-ntml-adv-tmi.md).

## Problem

VATUSA's traffic-management / flow tooling lives in an external tool (vatflow — the
modern successor to the older SimTraffic). Those capabilities sit outside the platform,
don't share identity/permissions/data, and expose no API to integrate against.

OIS brings this functionality **in-house as native features** in the `flow` domain — we
are not cloning or embedding vatflow, we reimplement its capabilities as our own
handlers so they share the platform's identity, permissions, and data, all driven off a
single ingested VATSIM data feed with a nav/winds/trajectory model for ETA prediction.

## Scope

The `flow` domain is a set of capabilities (separate handlers, not a single app port):

- **Flow Constrained Areas (FCAs)** — a controller-drawn polyline across airspace, with
  membership filters and a metering config, that the metering engine sequences crossing
  traffic against. Shared + server-side: one FCA set, visible to every controller.
- **Live traffic view** — current aircraft positions/phases from the VATSIM data feed,
  enriched with just enough of each flight plan to drive the facility map's color rules.
- **Shared reference routes** — named filed-route strings the nav engine resolves to a
  drawn track on read.
- **Runway balancer** — shared per-airport runway configuration; the arrival→runway
  assignment is computed live off the feed.
- **IDST (departure-release console)** — FCA-metered ground departures across a scope,
  with advisory/frozen EDCTs.
- **Facility map** — a per-facility public map with staff-editable, client-side aircraft
  color rules.
- **ATC overlay** — airport/TRACON/ARTCC positions derived from the feed.

## Data model

Domain schema: `flow` (sqlx migrations `0022`–`0025`, `0028`–`0029`, `0043`). All tables
carry `updated_at` (via the shared `platform.touch_updated_at()` trigger) and reference
`identity.users(id)` for author/editor columns.

### `flow.fca` — Flow Constrained Areas *(migration 0022, extended 0023)*

| column | type | notes |
| --- | --- | --- |
| `id` | text pk | `gen_random_uuid()` |
| `name` | text | |
| `color` | text | hex, default `#f59e0b` |
| `artcc` | text | owning facility (sidebar filter) |
| `points` | jsonb | `[lat, lon]` vertices — an open polyline (≥ 2 pts) |
| `dests` / `origins` / `fixes` | text[] | membership filters |
| `scope` | text[] | ARTCCs the FCA applies in (crossing must fall inside) |
| `min_fl` / `max_fl` | int null | FL band; null = SFC / UNL |
| `dir` | text | `any` \| `N` \| `S` \| `E` \| `W` |
| `mode` | text | `rate` \| `mit` metering mode |
| `rate` | int | arrivals/hour (0–240) |
| `mit` | int | miles-in-trail (0–200) |
| `enabled` | boolean | |
| `manual_order` | text[] | drag-to-reorder callsign list (migration 0023) |
| `manual_seq` | boolean | whether manual order is active (migration 0023) |

### `flow.fca_release` — frozen CFR releases per FCA *(migration 0023)*

One row per released crossing. When a controller issues a release, the flight's metered
crossing (`cta_ms`) and wheels-up/release time (`edct_ms`) are **frozen**; the metering
engine then treats it as a fixed constraint that unreleased ground traffic floats around.

| column | type | notes |
| --- | --- | --- |
| `fca_id` | text | → `flow.fca(id)` on delete cascade |
| `callsign` | text | pk `(fca_id, callsign)` |
| `cta_ms` | bigint | frozen metered crossing time, epoch ms |
| `edct_ms` | bigint | release / wheels-up time, epoch ms |

### `flow.route` — shared reference routes *(migration 0028, reshaped 0029)*

Named polylines controllers share on the FCA map. Originally hand-drawn `points`;
migration 0029 switched them to a filed-route **string** (`route`, plus optional `dep` /
`arr`) that the nav engine resolves to a track on read. The old `points` column remains
but is unused.

### `flow.runway_config` / `flow.runway_saved_config` — runway balancer *(migration 0024, extended 0025)*

`flow.runway_config` (pk `icao`): `active_ends` text[], `star_rules` jsonb
(`{STAR_base: end_id}`), `overrides` jsonb (`{callsign: end_id}`), `window_min`,
`custom_ends` jsonb (manually-added ends for airports the bundled dataset lacks).
`flow.runway_saved_config` (pk `icao, name`): named reusable configs (`payload` jsonb).

### `flow.facility_map_config` — per-facility color rules *(migration 0043)*

One row per ARTCC (pk `facility_id`, which **is** the owning ARTCC id, e.g. `ZDC`):
`rules` jsonb (ordered `ColorRule[]`), `default_color` text (hex; `''` = theme default).

## Permissions

Active `flow` permissions the handlers actually gate on:

| permission | purpose | holders |
| --- | --- | --- |
| `flow.fca.read` | IDST console (and any future gated flow read) | TMU staff, `NTMO` |
| `flow.fca.update` | create/edit FCAs, reorder, release, force a nav/winds refresh | TMU staff |
| `flow.fca.delete` | delete an FCA | TMU staff |
| `flow.route.update` | create/edit shared reference routes | TMU staff |
| `flow.route.delete` | delete a shared route | TMU staff |
| `flow.runway.read` | view the runway balancer + saved configs | TMU staff |
| `flow.runway.update` | edit runway config / STAR rules / assignments / saved configs | TMU staff |
| `flow.facility_map.update` | edit a facility map's aircraft color rules (**facility-scoped**; `facility_id` IS the ARTCC) | facility staff |

**Read exposure.** Most flow reads are currently **unauthenticated** — the public FCA
overview and facility map reuse the same handlers, so `list_fcas`, `fca_traffic`,
`fca_counts`, `list_routes`, `aircraft_route`, `list_traffic`, `data_status`,
`route_coverage`, and the ATC overlay carry no `RequirePermission`. IDST is the one flow
read that is gated (`flow.fca.read`). `resolve-routes` gates on `stats.data.read` (it is
consumed by the replay map). `data-refresh` gates on `flow.fca.update`.

**Vestigial catalog entries.** `crates/ois-core/src/catalog.rs` still lists
`flow.programs.{read,create,update,publish,delete}` and `flow.data.read` from the
original spec. **No handler references them** — they are dead catalog rows kept only so
the access editor doesn't lose them; the shipped gates are the `flow.fca.*` /
`flow.route.*` / `flow.runway.*` / `flow.facility_map.update` set above (seeded by the
`flow` migrations, not by `draft_new_permission_names`).

## API

Versioned REST under `/api/v1`, routed in `backend/src/router.rs`. Handlers in
`backend/src/handlers/flow.rs` unless noted.

### FCAs + metering

| method + path | permission | purpose |
| --- | --- | --- |
| `GET /flow/fcas` | none (public) | list FCAs |
| `POST /flow/fcas` | `flow.fca.update` | create an FCA (publishes `flow.fca`) |
| `PUT /flow/fcas/{id}` | `flow.fca.update` | edit an FCA |
| `DELETE /flow/fcas/{id}` | `flow.fca.delete` | delete an FCA |
| `GET /flow/fcas/{id}/traffic` | none | metered crossing candidates for one FCA |
| `PUT /flow/fcas/{id}/order` | `flow.fca.update` | set/clear the manual sequence |
| `POST /flow/fcas/{id}/release/{callsign}` | `flow.fca.update` | issue a release (RDY/SET); freezes `cta`/`edct`, publishes `flow.release` |
| `DELETE /flow/fcas/{id}/release/{callsign}` | `flow.fca.update` | clear a release; publishes `flow.release` |
| `GET /flow/counts` | none | live crossing counts per FCA |

### Routes, traffic, nav

| method + path | permission | purpose |
| --- | --- | --- |
| `GET /flow/routes` | none | shared reference routes, each resolved to a track |
| `POST /flow/routes` | `flow.route.update` | create a route |
| `PUT /flow/routes/{id}` | `flow.route.update` | edit a route |
| `DELETE /flow/routes/{id}` | `flow.route.delete` | delete a route |
| `GET /flow/aircraft/{callsign}/route` | none | one flight's resolved route + waypoints |
| `POST /flow/resolve-routes` | `stats.data.read` | batch-resolve filed routes for the replay overlay |
| `GET /flow/traffic` | none | live map traffic (`TrafficAircraft[]`) |
| `GET /flow/data-status` | none | nav + winds health (cycle, source, counts, refresh times) |
| `POST /flow/data-refresh` | `flow.fca.update` | force a nav + winds refresh |
| `GET /flow/route-coverage` | none | how much live filed traffic the nav engine resolves |
| `GET /flow/idst` | `flow.fca.read` | IDST departure-release board (see below) |
| `GET /flow/atc` | none | ATC overlay (`handlers/atc.rs`) |
| `GET /flow/facilities` | none | facility list for the flow map (`handlers/atc.rs`) |

### Runway balancer *(`handlers/runway.rs`)*

| method + path | permission | purpose |
| --- | --- | --- |
| `GET /flow/runway/{icao}` | `flow.runway.read` | live runway board + config |
| `PUT /flow/runway/{icao}` | `flow.runway.update` | edit config / STAR rules / overrides |
| `GET /flow/runway/{icao}/configs` | `flow.runway.read` | list saved configs |
| `PUT /flow/runway/{icao}/configs/{name}` | `flow.runway.update` | save a named config |
| `DELETE /flow/runway/{icao}/configs/{name}` | `flow.runway.update` | delete a named config |

### Public "my flight" lookup

`GET /api/v1/public/flight/{callsign}` (handler `flow::flight_advisory`, **no auth**):
everything currently affecting one callsign — its arrival GDP / ground stop / rate
program (with this flight's delay + EDCT) and every FCA it crosses (metered). `found` is
false if the callsign isn't live in the feed.

### `GET /flow/traffic` — enriched map traffic

`TrafficAircraft` now carries, in addition to position/`dep`/`arr`/`actype`:

- `star` — arrival gate / STAR base name (revision stripped), parsed from the filed route;
- `wake` — wake category (`L`/`M`/`H`/`J`);
- `flight_rules` — as filed (`I`/`V`/…);
- `filed_alt` — filed cruise altitude in feet (0 if unparseable).

These fields exist specifically to drive the facility map's client-side color rules.

## IDST — departure-release console

`GET /api/v1/flow/idst`, gated `flow.fca.read` (`handler flow::list_idst`). The console
aggregates FCA-metered **ground** departures across a scope so a controller can time
releases for FCA crossings (a native port of vatflow's idst view).

- **Scope** — comma-separated `airports`, `tracons`, and `artccs` query params, resolved
  to the union of member airport ICAOs. Empty scope → empty board.
- **Response** — `IdstResponse { unscheduled, released, metered_count, as_of }`. Each
  `IdstFlight` is one `(metering FCA, ground departure in scope)` row: `callsign`, `dep`,
  `arr`, `aircraft_type`, `status` (`ground`/`proposed`), `fca_id`/`fca_name`, `seq`,
  `delay_min`, `cross_time` (metered CTA), `edct`, `released`.
- **EDCTs** — for a released flight, `edct` is the frozen wheels-up from
  `flow.fca_release`. For an unreleased flight it is an **advisory** EDCT: the wheels-up
  that would hit the metered crossing, computed by backing the modeled transit
  (`eta − now`) out of the metered CTA — so the controller can preview a release before
  issuing it.
- Unscheduled flights sort by metered crossing (soonest first); released by frozen EDCT.

## FCA releases in the departures view

CFR releases stored in `flow.fca_release` (frozen `cta_ms`/`edct_ms`) also feed the TMU
**departures** view and IDST metering, so a release set on the FCA page surfaces
everywhere — even when the destination has no GDP program (e.g. KSAN metered by an FCA,
not a GDP). In `departures_response` (`handlers/feed.rs`), an FCA release for a pending
departure counts as a frozen CFR: the GDP-program CFR is preferred when present,
otherwise the flight falls back to the FCA's release time and is marked metered.
`POST`/`DELETE /api/v1/flow/fcas/{id}/release/{callsign}` publish the `flow.release`
realtime topic (the additive `/api/v1/ws` push hub) so those views nudge-and-refetch.

## Facility map

A per-facility **public** map (`/facility-map/$id` in the web app) with staff-editable,
client-side aircraft color rules. The map itself is public; only the color-rule config is
gated. Handlers in `backend/src/handlers/facility_map.rs`.

| method + path | permission | purpose |
| --- | --- | --- |
| `GET /api/v1/facility-map/{id}/config` | none (public read) | this facility's color rules |
| `PUT /api/v1/facility-map/{id}/config` | `flow.facility_map.update` (facility-scoped) | replace the color rules |

- **`id`** normalizes to an uppercase ARTCC code (2–4 alphanumerics); anything else is a
  400. `facility_id` **is** the ARTCC, so the scope check is direct — no owning-ARTCC
  lookup.
- **`editable` flag** — the public GET resolves the caller's scope
  (`access_repo::permission_scope(user, "flow.facility_map.update").allows(facility_id)`)
  and returns `editable` so the UI knows whether to show edit controls. `false` when
  signed out. The PUT re-checks the same scope and 403s if it fails.
- **Config shape** — `FacilityMapConfigBody { facility_id, rules, default_color, editable }`.
  `rules` is an ordered `ColorRule[]`; the first enabled rule whose conditions all match
  paints the aircraft, otherwise `default_color` (empty = the map's theme default).

### `ColorRule` + condition vocabulary

```
ColorRule { id, label, color (hex), enabled, conditions: RuleCondition[] }
RuleCondition { field, op, values[] }   // conditions within a rule are ANDed
```

- **field**: `arr` | `dep` | `star` | `type` | `wake` | `rules` | `alt`
- **op**: `eq` | `in` | `prefix` | `lt` | `gt` | `range`

The semantics live in the client rule engine; the backend stores rules opaquely and only
guards against obviously-bad sets (≤ 100 rules, ≤ 20 conditions each, bounded string
lengths). The fields map onto the enriched `TrafficAircraft` (`arr`/`dep`/`star`/`type`
=`actype`/`wake`/`rules`=`flight_rules`/`alt`=`filed_alt`).

## Data ingestion

A background poller (`backend/src/jobs`, `backend/src/feed`) fetches the VATSIM data feed
on an interval into an in-memory `Snapshot`; a nav (FAA NASR), winds, and trajectory
model back ETA/crossing prediction. All metering (`fca::meter`, the runway board, IDST,
the departures view) is computed **live** per request against the current snapshot — the
`flow` schema stores config and frozen releases, not traffic history. Persisted network
history lives in the separate `stats` domain.
