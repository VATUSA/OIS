# Network statistics & replay

Persists a continuous, US-scoped record of the VATSIM network so OIS can show **historical
statistics** and **replay** any past window as a time machine — scrub an event, watch the traffic,
the flow initiatives, and each flight's plan exactly as they were. The design goal is *faithful
replay at bounded storage cost*, which is why several layers of **data compression** sit between the
live feed and disk.

## Collection

A background collector (`backend/src/feed/stats/mod.rs`) reuses OIS's **single** in-memory feed
snapshot — no second HTTP poll. Each tick (`COLLECT_SECS = 10s`; the VATSIM feed itself refreshes
~every 15s, and a `source_timestamp` dedupe drops re-served snapshots) it decomposes the snapshot
into:

- **flight & controller sessions** — keyed by a stable 64-bit id derived from `(cid, logon_time)`
  (`session.rs`), so a reconnect continues the same session.
- a **position** row per airborne pilot (`stats.position`) — the only large table.
- a per-tick **network snapshot** (`stats.snapshot`) — connected/unique/pilots/controllers counts.

By default only **US-scoped** traffic is stored; while a capture window is open the scope filter is
relaxed so the whole network is recorded (see [Capture windows](#capture-windows)).

## Data model (`stats` schema, migrations 0039–0044)

- `stats.member` — per-CID roll-up.
- `stats.flight` — one row per pilot session: plan (dep/arr/type/route), summary
  (`duration_s`, `distance_nm`, `max_altitude`, `max_groundspeed`), and **`path_simplified`** (the
  compressed track — below). `revision_id` tracks the latest filed-plan revision.
- `stats.position` — the high-frequency position time-series (`session_id, ts, lat, lon, alt,
  heading, gs`). BRIN index on `ts` + a `(session_id, ts desc)` index. This is what compaction acts on.
- `stats.controller_session` — ATC/ATIS sessions (no positions — they don't move).
- `stats.snapshot` — per-tick network totals (read-time hourly roll-up; no continuous aggregate).
- `stats.capture` — a saved/open time window (below).
- `stats.winds` (0042) — one serialized winds-aloft snapshot per hourly refresh, `jsonb` (Postgres
  TOASTs it) — so historical ETAs are computed against the winds that actually applied.
- `stats.flight_plan` (0044) — flight-plan **revision history**, for temporally-faithful replay.

## Data compression

No TimescaleDB — retention is a plain-Postgres, saved-window-aware job. Compression happens in
layers, each trading fidelity for size only where it doesn't hurt:

1. **Scope filter at ingest** — only US traffic is stored outside capture windows, so the firehose
   is cut to the region OIS actually serves.
2. **Per-flight track summary (permanent, tiny).** When a flight closes, its full track is reduced
   with **Douglas–Peucker** line simplification (`feed/stats/geo.rs`, tolerance ≈ 0.01° ≈ 0.6 nm):
   cruise legs collapse to their endpoints while turns and climbs keep their points, coordinates
   rounded to 5 decimals. The result is stored as `stats.flight.path_simplified`
   (`[[ts,lat,lon,alt], …]` JSONB) and **survives all pruning** — the lightweight "Tier-1" track that
   backs the flight-detail map forever.
3. **Change-only flight plans.** `stats.flight_plan` inserts a new row **only when the plan
   revision changes** (`insert … on conflict (session_id, effective_from) do nothing`, guarded by a
   revision comparison in `repos/stats.rs`). An unamended flight stores exactly one plan; a diversion
   stores two. Near-zero cost for the common case.
4. **Age-banded downsampling.** Positions older than **`STATS_DOWNSAMPLE_AFTER_DAYS = 2`** days are
   thinned to **keep every 4th sample** (`STATS_KEEP_EVERY = 4`) per session — ~15s resolution drops
   to ~1 minute. Recent data stays full-fidelity; the trend-level past doesn't need every point.
5. **Prune horizon.** Positions and winds older than **`STATS_PRUNE_AFTER_DAYS = 14`** days are
   deleted outright (the Tier-1 simplified tracks remain). Published TM history (TMIs/GDPs/ground
   stops) is pruned on the same horizon.
6. **Capture guard.** Any row whose `ts` falls inside an **open or saved** `stats.capture` window is
   **exempt from both downsampling and pruning** — captured events keep full ~15s fidelity
   indefinitely (`CAPTURE_GUARD` in `repos/stats.rs`).
7. **Read-time thinning for replay.** `replay_positions(from, to, step_s)` returns **one sample per
   `(session, step_s-bucket)`** via `DISTINCT ON`, so a replay payload stays bounded even for a
   full-network event capture, without touching stored fidelity.

The compaction job (`backend/src/jobs.rs`, `spawn_stats_compaction`) runs **hourly**, applying the
downsample band then the prune horizon then the winds/TM-history prunes — each skipping capture-guarded
rows.

### Retention tiers at a glance

| Age | What's kept |
| --- | --- |
| 0–2 days | every sample (~15s), full fidelity |
| 2–14 days | 1-of-4 samples (~1 min) |
| > 14 days | raw positions dropped; only the Douglas–Peucker `path_simplified` per flight survives |
| **inside a saved capture** | **full fidelity, forever** (never downsampled or pruned) |

## Replay

Two entry points (`backend/src/handlers/stats.rs`):

- `GET /api/v1/stats/captures/{id}/replay` — replay a saved capture window.
- `GET /api/v1/stats/replay?from=&to=&step=` — replay any custom `[from, to]` window.

Reconstruction, per flight:

- **Track** — `replay_positions(from, to, step)` builds the thinned per-flight sample list
  (`ReplaySample { t, lat, lon, alt, heading, gs }`, where `t` = seconds from the window start). The
  frontend's `frameAt(clock)` interpolates between samples as you scrub.
- **Plan in effect at each instant** — `flight_plan_revisions(ids, from, to)` returns, per session,
  the revision in force at/before `from` plus every revision that took effect inside the window; the
  handler attaches them as `ReplayFlightBody.plans` (a `ReplayPlan` timeline). The frontend's
  `planAt(clock)` selects the right one, so a flight that amended KJFK→KBOS to KJFK→KPHL mid-route
  shows KBOS before the amendment and KPHL after — the route, arrival gate, and metering all reflect
  the plan that actually applied. Captures predating 0044 have no revisions and fall back to a single
  plan from `flights_meta`.
- **Historical ETAs** — winds are read from the `stats.winds` snapshot at/before the replay instant,
  so past ETAs use the winds that actually applied, not today's.

The **historical dashboard** (`/historical/dashboard`) and **replay map** (`/historical/replay`)
consume these; they deep-link a capture (or a `from`/`to` window), a board, and the scrubber instant.

## Capture windows

`stats.capture` marks a time window as `open`, `saved`, or `discarded`, optionally tied to a VATUSA
event. While **open**, the collector relaxes the US scope filter (`relax_scope`) so the whole network
is recorded; a **saved** window is protected from compaction forever (full fidelity). A capture
scheduler (`jobs.rs`) opens/closes event windows automatically; event capture + stats live in
migration 0040.

## Key files

Collector: `backend/src/feed/stats/{mod,session,geo,reconstruct}.rs`. Storage + compaction +
replay queries: `backend/src/repos/stats.rs`. Jobs: `backend/src/jobs.rs`
(`spawn_stats_compaction`, capture scheduler). Handlers: `backend/src/handlers/stats.rs`. Schema:
`backend/migrations/0039_stats.sql` (+ 0040 event capture, 0042 history/winds, 0044 flight-plan
revisions). Frontend: `web/src/pages/stats/{replay,dashboard,flight}.tsx`.
