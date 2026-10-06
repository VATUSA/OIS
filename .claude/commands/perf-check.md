---
description: Read-only performance check of the branch — blocking work on async threads, N+1 queries, EXPLAIN on new SQL, feed-poller and per-tick refetch cost.
argument-hint: "[base ref] (default: origin/next)"
---

You are running an on-demand performance check of the current branch. It is **read-only**: report
findings with evidence, change nothing. Every claim is measured or labeled as an estimate.

## Step 1 — What changed, and what calls it

`git diff origin/next...HEAD --stat`, then for each changed file name its callers: the route
(`backend/src/router.rs`), the job (`backend/src/jobs.rs`), or the feed path (`backend/src/feed/`).
How often a piece of code runs decides whether it matters: once per request, once per feed tick, or
once per aircraft per tick.

## Step 2 — Blocking work on the async runtime

CPU-heavy work with no `.await` in it, run inline in an async handler, starves the Tokio workers: a
burst of polling clients stalls the health check and the websocket keepalive too, and the process
looks hung. That is why route resolution and metering run under `tokio::task::spawn_blocking`
(`backend/src/handlers/flow.rs:2141` explains it; `handlers/feed.rs`, `handlers/gdp.rs` and
`handlers/runway.rs` follow the same pattern).

Flag a new or changed async handler that:

- loops over every aircraft in the feed snapshot, resolves routes, or runs the trajectory model
  (`backend/src/feed/trajectory.rs`) inline;
- calls blocking std I/O (`std::fs`, `std::thread::sleep`) or a synchronous library on the request
  path;
- holds a lock or an `ArcSwap` guard across a long computation instead of cloning the `Arc` out first.

## Step 3 — Queries

For every new or changed query in `backend/src/repos/`:

- **N+1** — a repo call inside a loop over rows or aircraft. One query with `= any($1)` or a join
  replaces it.
- **EXPLAIN it.** Against a local database with representative data (`just up`; see
  `.claude/rules/database-postgres.md` for booting a throwaway database), run
  `EXPLAIN (ANALYZE, BUFFERS)` on the new query with realistic parameters. Report the plan's shape: a
  sequential scan on a table that grows, a sort that spills, a nested loop over a large outer side.
  A table that grows without bound (audit logs, stats, replays) needs an index that matches the
  `WHERE` and `ORDER BY`, and a migration to add it.
- **Unbounded results** — a list endpoint with no `LIMIT` or paging over a growing table.
- **Pool pressure** — the pool defaults to 20 connections with a 10-second acquire timeout
  (`backend/src/state.rs:124`). Flag a request that holds a connection or transaction across an
  external HTTP call or a long computation, and a `join_all` of queries that can take the whole pool.

## Step 4 — Feed poller and per-tick cost

The poller (`backend/src/feed/mod.rs`, the `loop` in `poller`) fetches the VATSIM snapshot about every
15 seconds, applies it, and publishes `feed.tick`. **Feed functions have no DB handle**: they read
`AppState` caches behind `ArcSwap`, filled by `jobs.rs` workers. Flag:

- a database query or HTTP call added to the feed path, rather than a cache plus a refresh job;
- work added to every tick that is O(aircraft × something) without a reason, especially anything
  reaching the trajectory model, whose callers (FCA metering, airport-flow demand, runway ETE, sector
  occupancy; `git grep -n 'trajectory::'`) all run per tick or per request;
- a cache rebuilt on every tick when its inputs change daily.

Each tick also fans out to every connected browser: `web/src/lib/realtime.ts` refetches every
feed-derived query on `feed.tick` once its data is older than its `minGapMs`. A new feed-derived
endpoint costs (its handler time) × (open clients) every tick. Say which `FEED_KEYS` entry it joins
and whether its `minGapMs` is right.

## Step 5 — Web

- A query refetched on a tick or interval that renders a large list without virtualization.
- `useEffect` chains that refetch in a loop, or query keys that change identity every render.
- A large derived computation in render that `useMemo` (or the server) should own.

## Step 6 — Measure what you can

Where a claim matters, measure it rather than argue it: `EXPLAIN ANALYZE` timings; a timing probe
around the function in a test; request latency from the running backend (`just backend`, then time the
endpoint); `GET /metrics` for the HTTP-metrics histograms (`backend/src/metrics.rs`). Say what you
could not measure.

## Report

### Blocking work on async threads
### Query issues — N+1, plans (with the EXPLAIN output that shows it), missing indexes, unbounded reads
### Feed and per-tick cost
### Web
### Recommendations — ranked by impact (high / medium / low), each with its evidence

**This check does NOT** fix anything, add an index, run a load test, or touch production.
