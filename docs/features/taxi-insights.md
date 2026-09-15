# Taxi & pushback insights

> **Status: built.** A staff-facing browsable history over the taxi/pushback timing model's raw
> observations and derived estimates — the model itself (learned per-gate/type/runway durations)
> predates this view; this is visibility into it.

## Problem

The trajectory/ETA model needs real pushback and taxi durations per (airport, gate, aircraft type,
runway) to time ground movement accurately — a flat, airport-wide guess under-times a long taxi and
over-times a short one. That model already existed and learns from observed data, but there was no
way to see what it had actually learned, or to spot a combo running on too little data to trust.

## Scope

**Built:** two browsable views — raw **observations** (one row per recorded pushback/taxi) and
derived **estimates** (the model's current per-combo output), each filterable by airport (required),
gate, aircraft, runway, and time range, with an outlier flag on observations and a fallback-tier
filter on estimates.

## Data model

No new tables — reads over the pre-existing `stats.taxi_observation` rows and the existing
`taxi_estimate::estimate()` function; this feature only adds the browsing layer
(`backend/src/repos/taxi_insights.rs`, `backend/src/handlers/taxi_insights.rs`).

**Estimate tiers**, most specific first — a combo resolves to the most specific tier with enough
samples, falling back progressively:

1. `gate_type_runway` — exact (gate, aircraft type, runway) match.
2. `airport_runway` — same runway, any gate/type.
3. `airport` — airport-wide, ignoring gate and runway.
4. `default` — no matching samples; a fixed fallback duration.

Gate IDs are lowercase (`gen_random_uuid()::text`, from `flow.airport_gate`) — filtering on them
must **not** uppercase, unlike every other filter field in this handler (a real bug caught and
fixed during review: `norm_opt()` uppercased everything including `gate_id`, silently matching zero
rows; `norm_gate_id()` trims only).

## Permissions

`stats.data.read` — the same access as the rest of [Historical](../../docs-site/historical/overview.md).

## API

- `GET /api/v1/stats/taxi/observations` — filterable, paginated raw observations.
- `GET /api/v1/stats/taxi/estimates` — filterable, paginated derived estimates (airport required;
  optional `fallback_tier` filter, which **rejects** an unparseable value with 400 rather than
  silently treating it as "no filter").

## Discord

None — this is a web-only view.

## Open questions

None currently open; a few non-blocking performance notes exist in the review history
(`fetch_taxi_estimates` recomputes per combo via a full linear scan rather than caching, and
`distinct_combos`/`sample_pool` run sequentially rather than concurrently) — not urgent at current
data volumes.
