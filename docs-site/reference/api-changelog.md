# API changelog

Changes to the OIS API (`/api/v1`) that a tool calling it might notice: new endpoints, changed request or
response shapes, new error responses, and anything removed.

::: warning Nothing is stable before 1.0
Until OIS 1.0, any part of the API can change in any release, and entries here are informational only.
From 1.0, OIS stands behind a defined set of endpoints. Those change only in ways that keep existing
callers working. Anything retired is announced here and carries `Deprecation` and `Sunset` headers on
its responses for at least 30 days before it goes.
:::

Each entry names the endpoint, what changed, and whether an existing caller has to do anything.

## Unreleased (pre-1.0)

- **Changed: one trajectory per flight behind every time** (#813). `GET /api/v1/tmu/flow/{icao}`,
  `GET /api/v1/tmu/demand`, `GET /api/v1/tmu/departures/{dep}`, `GET /api/v1/flow/fcas/{id}/traffic`,
  `GET /api/v1/flow/counts`, `GET /api/v1/flow/idst`, `GET /api/v1/flow/traffic/projected`,
  `GET /api/v1/flow/aircraft/{callsign}/route` and the `/api/v1/stats/hist/flow`, `departures` and
  `runway` replays now time a flight the same way. A flight is `airborne` from 50 kt groundspeed at any altitude (airport flow
  used above 60 kt and 300 ft). A ground flight whose route does not resolve is timed on the great
  circle to its destination, padded by 1.12, so it now appears on the FCA routes and projected
  traffic, which skipped it. On the live routes a ground flight's taxi time comes from its stored
  departure runway. An unfiled cruise speed takes the altitude default instead of 420 kt, and a
  step-climb altitude such as `FL350/370` reads as 35,000 ft. Airport-flow MIT converts at the
  trailing flight's predicted speed 40 nm out (its descent speed there once it is inside 40 nm), not
  360 kt. Response shapes are unchanged; a caller needs do nothing.
- **Documented: a description on every response, and `null` last in nullable references** (#888).
  The OpenAPI document at `/docs/api/v1/openapi.json` is now built with utoipa 6. Each response that
  had an empty `description` carries its HTTP reason phrase (`OK`, `Unauthorized`, `Not Found`), as
  OpenAPI 3.1 requires, and a nullable reference lists the reference before `{"type": "null"}` in its
  `oneOf`. No request, response body or status code changes, so a caller needs to do nothing.
- **Changed:** `GET /api/v1/flow/data-status` (and the `POST /api/v1/flow/data-refresh` answer, which
  is the same `DataStatus`) gains `winds_source` (`rap` or `none`) and `winds_valid_at` (the time the
  RAP field is valid for; null in still air), and drops `winds_stations`: winds now come from NOAA
  RAP, which has no stations. A caller reading `winds_stations` must read `winds_source` instead.
  `FcaFlightDebug.headwind` is the mean headwind over the remaining route, each point read at the
  profile's altitude there. `winds_refreshed` no longer means a field is in use: after failed
  refreshes drop to still air it keeps the time the last field loaded, so read `winds_source`. (#836)
- **Changed: `GET /api/v1/flow/idst`, `GET /api/v1/flow/fcas/{id}/traffic` and
  `GET /api/v1/flow/counts`** list and count connected pilots with a flight plan only. A prefile is no
  longer an FCA candidate, so `FcaFlight.status` is `airborne` or `ground` and `IdstFlight.status` is always
  `ground`; neither carries `proposed`. `POST /api/v1/flow/fcas/{id}/release/{callsign}` answers `404`
  for a callsign that is only prefiled. A release already stored for a prefiled callsign is kept but
  appears on no FCA surface (IDST `released`, the traffic list, metering) until that pilot connects;
  `GET /api/v1/tmu/departures/{dep}` and sector demand still read it as the flight's locked wheels-up
  (#865). The FCA delays in `GET /api/v1/public/flight/{callsign}` and `GET /api/v1/me/flight` no longer
  count prefiles ahead. A caller that filtered `proposed` rows out needs do nothing; one that released
  prefiled flights must wait for the pilot to connect. (#834)
- **Documented: 403 and 404 on the advisory publish, cancel and delete routes** (#824).
  `POST /api/v1/tmu/advisories/{id}/publish`, `POST /api/v1/tmu/advisories/{id}/cancel` and
  `DELETE /api/v1/tmu/advisories/{id}` already answered 404 for an unknown id and 403 for an advisory
  outside the caller's facility scope; the OpenAPI document now lists both. Nothing changes for a caller.
- **Changed: release and CFR times are wheels-up, not gate times** (#815). `FlowFlight.etd` on
  `GET /api/v1/tmu/flow/{icao}` is a ground or prefiled flight's estimated wheels-up: now for a ground
  flight, or for a prefile its filed departure time (never earlier than now + 20 min, and now + 20 min
  when none parses), plus the ground allowance. It was the same time with no allowance. Every time backed
  out of it moves with it, so for a ground or prefiled flight each of these is one ground allowance later
  than before: `FlowFlight.cfr`, the EDCTs a GDP assigns, the `edct` a release stores when
  `POST /api/v1/flow/fcas/{id}/release/{callsign}` is sent without a `ready` time, and the advisory
  `edct` on `GET /api/v1/flow/idst`. Each is now the CTA less the airborne flight time.
  - `POST /api/v1/tmu/cfr` without a `ready_time` locks the flight's `FlowFlight.cfr`, else its
    `FlowFlight.etd`, and now only when the flow has neither (the flight is not listed, or is airborne).
    It locked `cfr` or now, so a ground flight bound for an airport with no program now gets its
    wheels-up estimate instead of the current time.
  - With a `ready_time` and an airport-flow program, `POST /api/v1/tmu/cfr` locks the later of that
    time and now, pushed later only if the flight's arrival would conflict. It now projects the arrival
    by the airborne flight time alone.
  - A release sent with a `ready` time still stores that time as its `edct`, and now stores a CTA one
    allowance earlier: the typed time plus the airborne flight time.
  - For a flight holding an issued CFR, `FlowFlight.sta` and `delay_min` are one allowance earlier: the
    locked wheels-up plus the airborne flight time.
  - A GDP's `max_enroute_min` tier counts a ground flight's airborne minutes, without its ground
    allowance, so the set of flights a GDP controls can change. That reaches
    `GET /api/v1/tmu/gdp/{id}/board`, `POST /api/v1/tmu/gdp/{id}/publish`, `PUT /api/v1/tmu/gdp/{id}`,
    `POST /api/v1/tmu/gdp/{id}/slots/{callsign}` and `POST /api/v1/tmu/gdp/{id}/compress`.

  Releases and CFRs stored before the change keep their stored value. The shapes are unchanged; a tool
  that read `etd` as an off-block time should read it as wheels-up.
- **New:** `PUT /api/v1/flow/sector-consolidations/{artcc}` changes several of one ARTCC's sector
  consolidations in one request. The body is `{ "into": { "<sector_id>": "<target_sector_id>" | null } }`:
  each sector is worked at the target given, or given its own row back for `null`. It needs
  `flow.sector_consolidations.update` for that ARTCC, like the single-sector `PUT` and `DELETE`, and is
  all or nothing: a self-reference, two keys naming one sector or more than 200 entries (400), an unknown or other ARTCC's
  sector (404) or a loop (409) writes none of it. Answers with the ARTCC's consolidations. Existing
  callers need do nothing. (#794)
- **New: `GET /api/v1/admin/access/vatusa-reset`, `POST /api/v1/admin/access/vatusa-reset` and
  `GET /api/v1/admin/access/vatusa-reset/runs/{id}`** (#795, #806). The dry run, the reset of every
  user's access to what VATUSA justifies, and the reset's result. Server admin only. The `POST` takes
  `AccessResetRequest` (`reason`) and answers at once with `202 AccessResetStarted` (`run_id`); the
  reset runs in the background and finishes whether or not the caller waits. A second `POST` while one
  runs gets `409` with the running run's `AccessResetStarted`. An unconfigured VATUSA
  (`vatusa_not_configured`), or a reset lock held with no run recorded (`reset_lock_busy`, try again),
  gets `503` `AccessResetFailure`, and nothing starts. Poll the run's `GET` for `AccessResetRun`: `status` is
  `running`, then `succeeded` with `result` (`AccessResetBody`) or `failed` with `failure`
  (`AccessResetFailure`: `error` is `vatusa_pull_failed`, `reset_incomplete`, `reset_failed` or
  `reset_interrupted`, with `message` and `users_reset`). The dry run reads the VATUSA data the last
  pull stored and the reset pulls fresh, so the two can differ if VATUSA changed in between. The reset
  keeps `system` grants, so the baseline `USER` and `SERVER_ADMIN` stay; a row of either written by
  hand as `manual` is removed like any other (#805). Additive; no existing caller changes.
