# AADC — Airport Arrival Demand Chart

> **Status: built.** A bucketed forward arrival-demand view for one airport, modeled on
> SIMTRAFFIC's AADC. Built as an extension of the existing live-flow computation
> (`feed::flow::compute`), not a parallel prediction path.

## Problem

The only forward-looking arrival-demand signal before this was a single `demand_60min` count. TMU
staff wanted to see the shape of a push arriving — how it's distributed over the next few hours,
and broken down by what kind of traffic is driving it — before it lands, not just a current number.

## Scope

**Built:** 15/30/60-minute bucketing over a fixed 4-hour forward window; breakdown by status
(airborne/ground/proposed), aircraft category (wake class), carrier, and arrival fix; the airport's
wind-favored AAR/ADR as a reference. A dedicated page and a configurable dashboard widget.

**Not built (out of proportion to the issue that shipped this):** a "Center" breakdown dimension —
this codebase has no "which ARTCC is currently working this aircraft in the air" concept, only an
airport→ARTCC mapping; building a real point-in-polygon lookup against live position was scoped out.
A Dfix (departure-fix) dimension was dropped per the same issue's own suggestion.

## Data model

No new tables — `bucket_aadc` (`backend/src/feed/flow.rs`) is a pure function over the same
`Vec<FlowFlight>` the live flow board already computes, so it inherits that computation's landed-
flight exclusion (`status != "arrived" && !excluded`) rather than reimplementing it. `FlowFlight`
gained one additive field, `category: Option<String>` (the wake class `L`/`M`/`H`/`J`, already
computed for metering but previously discarded).

Carrier is derived from the callsign's leading alphabetic prefix at read time (not stored); the
breakdown is capped to the 8 busiest carriers in the response window, with the rest folded into
`OTHER` so the payload stays bounded at a busy international airport.

AAR/ADR resolution ports the client's existing wind-favored-config matching
(`web/src/lib/airport-configs.ts`'s `matchConfig`/`inWindRange`) to Rust
(`repos::airport_configs::favored_config`/`in_wind_range`), combined with the existing forecast-wind
lookup (`feed::forecast::wind_at`) already used by the event-planning rate predictor.

## Permissions

Reuses `tmu.program.read` — the same permission the live arrival-flow board already requires. No
new permission was added; AADC is a view over data that permission already gates.

## API

`GET /api/v1/tmu/flow/{icao}/aadc?bucket_min=15|30|60` — returns `AadcResponse` (`icao`,
`bucket_min`, `aar`, `adr`, `config_id`, `generated_at`, and `buckets: AadcBucket[]`, each bucket
carrying `start`/`end`/`total` plus `by_status`/`by_category`/`by_carrier`/`by_afix` count maps).

## Discord

None — this is a web-only view.

## Open questions

- Whether a Center dimension is worth the ARTCC-boundary lookup it would need, if a future issue
  wants it (flagged as a candidate follow-up, not filed as of this doc).
