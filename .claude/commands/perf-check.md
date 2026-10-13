---
description: Read-only performance check of the branch — blocking work on async threads, N+1 queries, EXPLAIN on new SQL, feed-poller and per-tick refetch cost.
---

You are performing an on-demand performance check on the current branch. This is a read-only assessment — report findings, do not auto-fix. Every claim is measured or labeled as an estimate.

## Step 1: Identify Changed Endpoints

1. Run `git diff origin/next...HEAD --stat` to see changed files.
2. For each changed file, name what runs it: the route (`backend/src/router.rs`), the job (`backend/src/jobs.rs`), the feed path (`backend/src/feed/`), or the web query that calls it (`web/src/lib/`).
3. Say how often each runs, because that decides whether it matters: once per request, once per feed tick, or once per aircraft per tick.

## Step 2: Blocking Work on Async Threads

CPU-heavy work with no `.await` in it, run inline in an async handler, starves the Tokio workers: a burst of polling clients stalls the health check and the websocket keepalive too, and the process looks hung. That is why route resolution and metering run under `tokio::task::spawn_blocking` (the doc comment on `metered_flights` in `backend/src/handlers/flow.rs` explains it; `fca_counts` there, and `handlers/feed.rs`, `handlers/gdp.rs` and `handlers/runway.rs`, follow the same pattern).

Flag a new or changed async handler that:
1. Loops over every aircraft in the feed snapshot, resolves routes, or runs the trajectory model (`backend/src/feed/trajectory.rs`) inline.
2. Calls blocking std I/O (`std::fs`, `std::thread::sleep`) or a synchronous library on the request path.
3. Holds a lock or an `ArcSwap` guard across a long computation instead of cloning the `Arc` out first.

## Step 3: Query Analysis

For each new or changed query in `backend/src/repos/`:

1. Check for **N+1 queries** — a repo call inside a loop over rows or aircraft. One query with `= any($1)` or a join replaces it.
2. Check for **missing indexes** — a new `WHERE`, join or `ORDER BY` on a column without one. A table that grows without bound (audit logs, stats positions, replays) needs an index matching the `WHERE` and `ORDER BY`, and a migration to add it.
3. **EXPLAIN it.** Against a throwaway database with representative data (`.claude/rules/database-postgres.md` → "Running against a real database"; never the shared dev database in `.env` for anything that writes), run `EXPLAIN (ANALYZE, BUFFERS)` on the new query with realistic parameters. Report the plan's shape: a sequential scan on a table that grows, a sort that spills, a nested loop over a large outer side. Quote the row counts beside the timing.
4. Check for **unbounded results** — a list endpoint with no `LIMIT` or paging over a growing table.
5. Check for **pool pressure** — the pool defaults to 20 connections (`DATABASE_MAX_CONNECTIONS`) with a 10-second acquire timeout (the pool setup in `AppState::from_env`, `backend/src/state.rs`). Flag a request that holds a connection or transaction across an external HTTP call or a long computation, and a `join_all` of queries that can take the whole pool.

## Step 4: Caching and the Feed Poller

The poller (`poller` in `backend/src/feed/mod.rs`) fetches the VATSIM snapshot about every 15 seconds, applies it, and publishes `feed.tick`. **Feed functions have no DB handle**: they read `AppState` caches behind `ArcSwap`, filled by `jobs.rs` workers, and a write handler force-reloads its cache so an edit applies at once.

1. Flag a database query or HTTP call added to the feed path, rather than a cache plus a refresh job.
2. Flag work added to every tick that is O(aircraft × something) without a reason, especially anything reaching the trajectory model, whose callers (`AGENTS.md` § The trajectory / ETA model; re-derive with `git grep 'trajectory::' -- backend/src`) run per tick or per request.
3. Flag a cache rebuilt on every tick when its inputs change daily, and a cache whose write handler does not force-reload it (stale until the next refresh).
4. **Per-tick refetch.** Each tick fans out to every connected browser: `web/src/lib/realtime.ts` refetches every feed-derived query on `feed.tick` once its data is older than its `minGapMs`. A new feed-derived endpoint costs (its handler time) × (open clients) every tick. Say which `FEED_KEYS` entry it joins and whether its `minGapMs` is right.

## Step 5: Frontend Performance

1. A query refetched on a tick or interval that renders a large list without virtualization.
2. `useEffect` chains that refetch in a loop, or query keys that change identity every render.
3. A large derived computation in render that `useMemo` (or the server) should own.

## Step 6: Measure What You Can

Where a claim matters, measure it rather than argue it: `EXPLAIN ANALYZE` timings; a timing probe around the function in a test; request latency from the running backend (`just backend` on a throwaway database, then time the endpoint); `GET /metrics` for the HTTP-metrics histograms (`backend/src/metrics.rs`). Say what you could not measure.

## Step 7: Report

Compile findings into:

### Blocking Work on Async Threads
- Handlers doing CPU-heavy or blocking work inline

### Query Issues
- N+1 queries found
- Plans (with the EXPLAIN output that shows each), missing indexes
- Unbounded reads, pool pressure

### Feed and Per-Tick Cost
- Work added to the tick, caches rebuilt too often or never force-reloaded
- New feed-derived queries and their `minGapMs`

### Frontend Performance
- Large lists, refetch loops, unstable query keys

### Recommendations
- Prioritized list of improvements with estimated impact (high/medium/low), each with its evidence

**This check does NOT:**
- Auto-fix issues or add an index
- Run load tests
- Touch production or write to the shared dev database
- Modify any files
