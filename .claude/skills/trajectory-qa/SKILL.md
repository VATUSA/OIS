---
name: trajectory-qa
description: How to check, test and QA a change to OIS's trajectory/ETA model and everything that times against it (FCA metering, airport-flow demand and the GDP and CFR code built on it, runway ETE, sector occupancy, projected traffic, flight advisories), its inputs (route path, taxi allowance, aircraft profiles, winds, nav) and the feed caches behind them. Use before reporting any such change as done, when reviewing one (/code-review Phase 6), or when an operator says an ETA, a metering order or a demand count looks wrong. Covers the caller map, targeted test runs, hand-worked numbers, parity against next through the historical replay and the live feed, and where the model is documented.
---

# Trajectory QA

A green nextest run proves the predictor agrees with its fixtures. It does not prove that every surface built on it still shows an operator the ETA, the order and the count it showed yesterday. `backend/src/feed/trajectory.rs` is the **single** ETA predictor, so one change moves every caller at once; this skill is how you prove what it moved, caller by caller.

`.claude/rules/feed-and-trajectory.md` is the reference for how the system is meant to behave, and `.claude/rules/domain-lessons.md` records what it got wrong before (both path-scoped: read them in full first). `AGENTS.md` § The trajectory / ETA model lists the callers. This file is the procedure for checking that they still behave.

---

## 1. How the system fits together

```
 VATSIM data feed (~15 s)                         jobs.rs refresh workers
   │ poller (feed/mod.rs)                           │ nav · airports · winds · aircraft profiles
   ▼                                                ▼
 feed snapshot ─────────────► AppState caches behind ArcSwap (state.rs)
                                    ▲  force-reload on write (handlers/aircraft_profiles.rs)
                                    │
   feed/fca.rs route_path ──┐       │       feed/taxi_estimate.rs ground allowance
                            ▼       │                  │
                feed/predict.rs  profile_from_here · arrival_eta ◄──┘ (via feed/flow.rs)
                            │
                            ▼
                feed/trajectory.rs  VerticalProfile · AircraftProfile · capped_cruise_tas
                            │
   ┌────────────────────────┼──────────────────────────┬──────────────────────────────┐
   ▼                        ▼                          ▼                              ▼
 handlers/flow.rs        feed/flow.rs compute       feed/runway.rs               feed/sector_tracks.rs
 metered_flights,        ├► handlers/feed.rs        collect_arrivals             project_tracks
 projected_traffic,      │  flow_from_data          └► handlers/runway.rs        └► handlers/sector_demand_cache.rs
 build_flight_advisory   │  → /tmu/flow/{icao},        build_board                  → /flow/sector-demand/{artcc}
 → /flow/fcas/{id}/      │    /aadc, /tmu/demand       → /flow/runway/{icao}
   traffic,              ├► handlers/gdp.rs live_inbounds (GDP)
   /flow/traffic/        └► ready_time_slot → handlers/feed.rs issue_cfr (CFR)
   projected,
   /public/flight/{cs}, /me/flight

 stats.flight · stats.position · stats.controller_session · stats.winds
   └► reconstruct_at (feed/stats/reconstruct.rs) ► handlers/stats.rs hist_flow / hist_runway
        └► the same flow_from_data / build_board_from, at a past instant T
```

All routes are under `/api/v1`. The feed has no DB handle: everything above the predictor reads caches, and the historical handlers rebuild a snapshot from the `stats` tables and run the same compute. Winds at T come from `stats.winds`; nav is whatever cycle the running backend loaded.

The list goes stale. Re-derive it before you rely on it, keeping non-test call sites (both greps also match doc comments, `#[cfg(test)]` code and constant imports):

```bash
git grep -n 'trajectory::' -- backend/src
git grep -n 'predict::' -- backend/src
git grep -n -e 'flow::compute' -e 'collect_arrivals(' -e 'project_tracks(' -e 'ready_time_slot(' -e 'metered_flights(' -- backend/src
```

## 2. Where the knowledge lives, in trust order

When two sources disagree, the one higher in this list wins, and the lower one is a defect to fix.

1. **Code.** `backend/src/feed/trajectory.rs` (the model, `AircraftProfile`, `ProfileTable`, `MAX_PLAUSIBLE_TAS`), `feed/predict.rs` (the shared prediction service), `feed/fca.rs`, `feed/taxi_estimate.rs`, and each caller in section 1. Print a resolved value rather than reading its default: the loaded aircraft profiles come from `GET /api/v1/flow/aircraft-profiles`, the nav cycle and winds freshness from `GET /api/v1/flow/data-status`.
2. **`.claude/rules/feed-and-trajectory.md`** and **`.claude/rules/domain-lessons.md`**.
3. **Public docs** (`docs-site/`): `planning/aircraft-profiles.md`, `tmu/fcas.md`, `tmu/releases.md`, `tmu/idst.md`, `tmu/restrictions.md`, `dashboard/boards.md`. These are what operators read. A number you change in code that appears here must change here too.
4. **Internal docs** (`docs/features/flow.md`, `aadc.md`, `stats-replay.md`). Use them for intent, never for current behavior: a module doc comment can lag the code it heads.

## 3. The checks, and when each is required

| Tier | Required when the change touches |
| --- | --- |
| A. Targeted tests | always |
| B. Every caller, named | `trajectory.rs`, `predict.rs`, `fca.rs`'s route path, `taxi_estimate.rs`, `AircraftProfile` or the profile table, winds or nav inputs |
| C. Hand-worked numbers | a formula, a constant, a clamp, a threshold, a default |
| D. Same results as `next` on recorded traffic | anything reaching airport flow (and GDP or CFR) or runway ETE |
| E. Same results as `next` on the live feed | anything reaching FCA metering, sector demand, projected traffic or flight advisories |
| F. Both scope paths | a metering, FCA, GDP, CFR, runway or sector-demand handler |
| G. Docs cross-check | any number or behavior an operator can see |

Report which tiers ran and which you skipped and why. A tier you did not run is not a tier that passed.

### A. Targeted tests

```bash
cargo nextest run -p ois-backend -E 'test(/^feed::(trajectory|predict|fca|taxi_estimate)::/)'
cargo nextest run -p ois-backend -E 'test(/^feed::(flow|runway|sector_tracks|sector_load|gdp)::/)'
cargo nextest run -p ois-backend -E 'test(/^handlers::(flow|feed|gdp|runway|stats|sector_demand_tests|flow_event_fca_visibility_tests)::/)'
```

The `handlers::` set holds `#[sqlx::test]`s, so it needs `DATABASE_URL` pointing at a Postgres it may create databases in. Read nextest's summary line, not the exit code. A filter that matches nothing reports `0 tests run`: that is a typo, not a pass. These are targeted runs; the full gate (`just test`) still runs once per SHA.

### B. Every caller, named

Re-derive the callers (section 1). For each one the change reaches, write one line: the caller, the surface an operator sees it on, the test that proves it still behaves (cite `file:line`), or "no test" as a finding. A test of `trajectory.rs` alone does not cover a caller: the regression OIS ships most often is a caller drifting while the unit stays green. Inputs count too: a change to `fca::route_path` or the taxi allowance moves every ETA built on them without touching `trajectory.rs`.

### C. Hand-worked numbers

For a formula or constant change, add or update a unit test whose expected value you worked out by hand from the requirement, with the arithmetic in a comment, never computed by calling the code under test. Pin a constant with absolute fixtures that straddle the documented value plus an assert that the constant still sits between them, then mutate the constant itself with `bin/mutate.sh` and watch the test go red. Test the far side of every clamp and band (`MAX_PLAUSIBLE_TAS`, the groundspeed clamp in `effective_gs`, the anchoring bias clamp).

### D. Same results as `next` on recorded traffic

The historical handlers run the live compute over a snapshot rebuilt from the `stats` tables at instant `T`, with the winds stored for `T`. Two backends, one built from `next` and one from your branch, over the same recorded window, must agree everywhere the change did not mean to move. This needs a second worktree on `origin/next` and a second backend build; `bin/setup-worktree.sh` seeds its `target/` from the primary checkout's, so that rebuilds only the workspace crates.

**Never boot either backend against the shared dev database in `.env`**: startup applies migrations, and the database carries other branches' history. Each backend gets its own throwaway database (`.claude/rules/database-postgres.md` → "Running against a real database"), on its own spare port.

1. **Pick a window.** A busy hour for one or two arrival airports the change should affect, and one it should not. Check the shared dev database holds positions for it (read-only):

   ```sql
   select count(*) from stats.position where ts between '<T - 30 min>' and '<T>';
   ```

   No rows means the tier cannot run here: say so.

2. **Boot both backends once** on their throwaway databases, so each applies its own migrations, then stop them.

3. **Copy the window into each throwaway database**, read-only from the dev database. `stats.flight`, `stats.controller_session` and `stats.winds` around `T`, and `stats.position` for the window only (it is the large one). For example, per table:

   ```bash
   psql "$DATABASE_URL" -c "\copy (select * from stats.position where ts between '<T - 30 min>' and '<T>') to stdout" \
     | psql postgres://ois:ois@127.0.0.1:5432/<throwaway> -c "\copy stats.position from stdin"
   ```

   If the branch changes a `stats` table, select the columns both schemas share explicitly instead of `*`.

4. **Mint a reader in each throwaway database.** The historical routes take `stats.data.read`; grant it directly and create a session, the way `scope_test_support.rs` does in tests:

   ```sql
   insert into identity.users (cid, full_name, display_name) values (999999, 'QA', 'QA') returning id;
   insert into access.user_permissions (user_id, permission_name, granted, artcc_id, source)
     values ('<id>', 'stats.data.read', true, null, 'manual');
   insert into identity.sessions (session_token, user_id, kind, expires_at)
     values ('<random token>', '<id>', 'web', now() + interval '1 hour');
   ```

5. **Boot both again and confirm they loaded the same nav cycle**: `GET /api/v1/flow/data-status` on each, `nav_cycle` equal. A different cycle changes routes and ETAs for a reason unrelated to your change.

6. **Query the same instants on both and diff:**

   ```bash
   curl -s -H 'Cookie: ois_session=<token>' "http://127.0.0.1:<port>/api/v1/stats/hist/flow/<ICAO>?at=<epoch>" | jq -S . > <scratch>/<side>-flow-<ICAO>-<epoch>.json
   curl -s -H 'Cookie: ois_session=<token>' "http://127.0.0.1:<port>/api/v1/stats/hist/runway/<ICAO>?at=<epoch>" | jq -S . > <scratch>/<side>-runway-<ICAO>-<epoch>.json
   diff <scratch>/next-flow-<ICAO>-<epoch>.json <scratch>/branch-flow-<ICAO>-<epoch>.json
   ```

   Take three or four instants across the window. Every difference is a finding until you can name the line of your diff that intends it; for an intended change, show the before and after for a few flights and say why the new number is right.

### E. Same results as `next` on the live feed

FCA metering, sector demand, projected traffic and flight advisories have no historical route. Run both backends (as in D: throwaway databases, spare ports) at the same time against the live feed, so each polls the same VATSIM snapshots.

- Create the same FCA on both with the same request body (`POST /api/v1/flow/fcas`, `flow.fca.update`), or the same sector configuration, before comparing.
- Compare only responses taken when both report the same `source_timestamp` on `GET /api/v1/feed/status`, and the same `nav_cycle` on `GET /api/v1/flow/data-status`.
- Diff `GET /api/v1/flow/fcas/{id}/traffic`, `/api/v1/flow/sector-demand/{artcc}`, `/api/v1/flow/traffic/projected` and `/api/v1/public/flight/{callsign}` as in D.

Each backend fetches its own winds, so this tier is noisier than D: a residual difference is explained (a different winds fetch, a flight that logged on between polls) before it is called noise, and re-checked on the next tick.

### F. Both scope paths

A national-scope and a facility-scope principal are two paths. For a changed handler, a router-level test (`backend/src/scope_test_support.rs`: `seed_user`, `grant`, `deny_scoped`, `session_cookie`, `send`, `send_json`) shows the facility-scoped caller sees its own ARTCC's result, is refused another ARTCC's with 403, and that a caller without the permission gets 401. A `SERVER_ADMIN` caller passes every check and proves nothing about one.

### G. Docs cross-check

When a number or behavior an operator can see changes, grep for the old value before calling it done:

```bash
git grep -n "<old value>" -- docs-site docs .claude/rules AGENTS.md CLAUDE.md
```

---

## Commands you will reach for

| Command | Use |
| --- | --- |
| `git grep -n 'trajectory::' -- backend/src`, `git grep -n 'predict::' -- backend/src` | Re-derive the callers (tier B). |
| `cargo nextest run -p ois-backend -E 'test(/^feed::trajectory::/)'` | One module's tests (tier A); widen the regex as in A. |
| `bin/mutate.sh FILE OLD NEW -- COMMAND` | Mutate a constant or comparison and run its killing test; restores the file (tier C). |
| `GET /api/v1/flow/data-status` | Nav cycle, winds stations and freshness; public. |
| `GET /api/v1/feed/status` | The feed snapshot's `source_timestamp`, to pair live comparisons. |
| `GET /api/v1/flow/aircraft-profiles` | The profile table the backend actually loaded. |
| `GET /api/v1/stats/hist/flow/{icao}?at=<epoch>`, `GET /api/v1/stats/hist/runway/{icao}?at=<epoch>` | The live compute replayed at a past instant (tier D); `stats.data.read`. |
| `just backend`, `just web` | Run the stack to look at the page an operator uses, on a throwaway database. |

## Cleanup

Drop both throwaway databases and stop both backends when done (`.claude/rules/database-postgres.md`). The comparison files live in your scratchpad, never in the worktree. Nothing in this procedure writes to the shared dev database: if a step would, stop.
