# Sim traffic

## Problem

The audit calls for migrating SimTraffic functionality into the site — but the decision is to **build our own**, not use
SimTraffic. This is the traffic data layer that feeds
[flow.md](flow.md) and the TMU average-delay page.

## Scope

Phase 1 decision needed. Core question: where does traffic/timing data come from (VATSIM data feed ingestion, our own
tracker) and what granularity the flow + delay tooling needs.

## Data model *(placeholder)*

- Traffic samples: aircraft, position/phase, timestamps (gate-out, wheels-up, entry, landing) — the inputs to
  average-delay computation.

## Permissions

Reads likely public/`flow.data.read`; ingestion is a backend job, not a user permission.

## Open questions

- Data source and ingestion cadence (a background job in `backend/src/jobs`).
- What metrics the flow + delay features actually require, to bound the schema.
- Retention of raw samples vs. rollups.
