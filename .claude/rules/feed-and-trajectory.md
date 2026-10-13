---
paths:
  - "backend/src/feed/**"
  - "backend/src/handlers/{flow,feed,gdp,runway,sector_demand,sector_demand_cache,flight_exclusions,aircraft_profiles,airport_configs,sector_limits,sector_consolidations,tmu,restriction_artcc,taxi_insights}.rs"
  - "backend/src/handlers/{flow_event_fca_visibility,sector_demand,sector_limits,sector_consolidations}_tests.rs"
  - "backend/src/repos/{flow,gdp,tmu,runway,aircraft_profiles,flight_exclusions,sector_limits,sector_consolidations,departure_runway,taxi_insights,airport_configs}.rs"
  - "backend/src/{tmi,advisory,state}.rs"
  - "fixtures/**"
---
# Feed & Trajectory Rules (not loaded at startup; read in full before touching the feed, the trajectory/ETA model, metering, flow or TMIs, or explaining an ETA)

The live feed, the one ETA predictor and everything built on it (FCA metering, airport flow, GDPs,
runway ETE, sector demand) are the project's most cross-cutting code. A change to one input moves
every surface that times an aircraft. These rules capture what no single file shows. The facts are
`AGENTS.md` § The trajectory / ETA model and § The live feed subsystem; the defects are
`domain-lessons.md`.

---

## The paths every change must exercise

1. **Each trajectory caller is its own path.** A change to the model, its inputs or a shared helper
   is verified at every caller below, not reasoned about from one.
2. **Airborne and not-yet-airborne are two paths.** `predict::arrival_eta` treats `gs >= 50` as
   airborne: timed from its position along the trimmed remaining route, anchored to its observed
   groundspeed. A ground or prefiled flight is timed from its departure field over the whole route,
   with `GROUND_TAXI_SEC` (8 min) or the learned ground allowance added, and `GROUND_ROUTE_FACTOR`
   (1.12) when its route does not resolve. Prefiles need their departure resolved
   (`prefile_position`); a `(0, 0)` placeholder once timed KJFK→KDCA as ~4,900 nm (#225).
3. **A national-scope and a facility-scope principal are two paths** for every write here
   (`secure-coding.md` → "Access Control").

## The feed subsystem
- The poller (`feed::spawn_poller`) fetches the VATSIM data feed every `FAST_POLL_SECS` (2 s) and
  backs off to `STALE_POLL_BACKOFF_SECS` (30 s) after `MAX_CONSECUTIVE_STALE_POLLS` polls with no new
  publish. It installs a new `Arc<Snapshot>` in `FeedInner` behind a tokio `RwLock`.
- A reader clones the `Arc<Snapshot>` and drops the lock before any CPU work, then runs the work under
  `spawn_blocking`. Holding the read guard across route resolution stalls the poller's writes.
- **The publish is the clock.** `Snapshot::source_timestamp` changes only on a new VATSIM publish; a
  poll that repeats a publish installs a new `Snapshot` with the same data. Key anything derived on
  the publish, not the `Arc` (`handlers/sector_demand_cache.rs`).
- Each replica polls on its own and raises `topic::FEED_TICK` with `publish_local` once per new
  publish; the tick never crosses replicas (#648).
- **The prediction and metering code has no DB handle.** `trajectory`, `predict`, `flow`, `fca`,
  `runway`, `sector_tracks` and `gdp` take tables, never a `PgPool`. Anything they need from the
  database is an `AppState` cache behind `ArcSwap`, loaded by a `jobs.rs` refresh pass and
  force-reloaded by its write handler (`code-quality.md` → "Caching"). The collectors that live
  under `feed/` (`stats`, `delays`, `taxi_observations`, `events`, `vatusa`) hold a pool and write;
  they are not on a request's prediction path.
- Per-request inputs that are not cached (excluded callsigns, locked wheels-up times) are read by
  the handler before the blocking task and passed in.

## The trajectory / ETA model

`feed/trajectory.rs` is the single predictor; `feed/predict.rs` is the single service every surface
calls to resolve a route and time it.

- `VerticalProfile::build` integrates climb (per-band IAS schedule), cruise (TAS or Mach) and descent
  to a real top-of-descent, with an ISA Mach↔TAS crossover. Distances are nm to destination; times
  are seconds; altitudes are feet.
- The aircraft's profile resolves exact ICAO type → wake class → default (`ProfileTable::resolve`);
  `resolve_label` names the tier for the debug view.
- Filed cruise TAS is capped by the profile (`capped_cruise_tas`); above `MAX_PLAUSIBLE_TAS` (700 kt)
  or below 60 kt it is replaced by an altitude default. Filed altitude junk parses to 35,000 ft.
- Wind: `Winds::route_headwind` along the path at cruise altitude; `None` means still air, never an
  error. `effective_gs` clamps groundspeed to 0.4–1.6 × TAS.
- Anchoring scales every predicted groundspeed by observed/predicted only for an aircraft established
  at cruise (≥ 250 kt, within 300 ft of cruise), clamped to ±15 % (#313).
- **The arrival field elevation is resolved in one place**, `predict::profile_from_here`. No caller
  passes it, so none can drop it and time a sea-level descent (#411).
- **The profile's samples are the truth.** `ground_speed_at` and `time_between` read the same samples;
  never correct one without the other (#361).

## Every caller of the model

When adding or changing ANYTHING the model reads (a profile field, a speed rule, wind, the route
resolver, the ground allowance), verify every caller in the same change. Silent drift between surfaces
is the default failure mode: one aircraft showing two ETAs.

1. **Airport-flow demand**: `feed::flow::compute` (via `predict::arrival_eta`), behind
   `handlers/feed.rs` and `handlers/gdp.rs`; `feed::flow::ground_estimate` for proposed departures
2. **Runway ETE**: `feed::runway::collect_arrivals`
3. **Sector occupancy and demand**: `feed::sector_tracks::project_tracks`, cached per publish by
   `handlers/sector_demand_cache.rs`
4. **FCA metering**: `handlers::flow::build_candidates`, two sites (live pilots and prefiles), each
   feeding `MeterInput.cross_speed` from `predict::eta_along_route`
5. **Map traffic and the per-fix debug table**: `handlers::flow::project_traffic`, `fix_predictions`,
   `prefile_fix_predictions`
6. **Inputs, not callers**: `fca::route_path` (the route every surface times along) and
   `feed/taxi_estimate.rs` via `feed::flow::resolve_ground_allowance_sec` (the ground allowance). A
   change to them moves every ETA built on them.
7. **Carry the `ProfileTable` only**: `state.rs`, `jobs.rs` (its refresh), `repos/aircraft_profiles.rs`,
   `handlers/aircraft_profiles.rs` (reload on write), `handlers/sector_demand_cache.rs` (a cache key),
   `scope_test_support.rs`

The list goes stale. Re-derive it before relying on it with `git grep 'trajectory::' -- backend/src`
and `git grep 'predict::' -- backend/src`, keeping only non-test call sites.

**Each site gets its own wiring test, and each is mutated on its own.** On #355 the wiring test seeded
only live pilots, so reverting the prefile site alone passed the whole suite;
`handlers::flow::mit_cross_speed_wiring_tests` now mutates each site independently.

## Metering

- **FCA metering** (`fca::meter`): converts a configured MIT to a time gap with
  `MIT ÷ cross_speed`, where `cross_speed` is the groundspeed **at the crossing fix** from the profile,
  not cruise (#355). The gap is frozen at release.
- **Releases** space against the ladder's metered crossing times, not raw ETAs, through the same
  `manual_order` the ladder uses, plus `RELEASE_MARGIN_MS` (15 s) on the RDY path only (#356). A SET
  (a controller's explicit time) is an instruction, not a slot search.
- **Programs and GDPs**: `feed::flow::apply_metering` meters arrival demand against the program's AAR;
  `feed::gdp::ration_by_schedule` assigns CTAs by RBS. Airborne and out-of-scope flights are exempt
  but still consume a slot; control times frozen at publish override fresh ones.
- **Flight exclusions** are per-ARTCC rows, but `all_excluded_callsigns` drops the callsign from every
  surface nationally (map, FCA lists, metering, counts, AADC demand). A new surface that lists or
  meters aircraft applies them, with a test that an excluded callsign is gone (#342).
- **Visibility**: an unpublished or archived event FCA is hidden from non-planners on every route
  that reads or operates it, answering 404 (`require_visible_event_fca`, #746; public traffic route,
  #586).

## TMIs and advisories

- `tmi.rs` encodes the structured restriction into the raw NTML line and its plain-English `decoded`
  form; `ntml_line` is the one place that assembles the full row the channel carries (#436). The codec
  is one-way.
- `advisory.rs` renders a structured advisory into the vATCSCC document; the reference documents in
  `fixtures/*-reference.json` are what its tests assert against.
- A TMI derives `decoded`; an advisory takes it from the caller. A change mirrored from one to the
  other diffs the two request types first (#488).
- Publish and cancel enqueue `tmi_publish` / `tmi_cancel` / `adv_publish` / `adv_cancel` in the same
  transaction as the state change. An unmapped channel skips the post and warns (`skipped_post`).
- The owning ARTCC of a restriction is derived at read time from the facility map
  (`restriction_artcc`, #405), never stored.
- Event FCAs and TMI packages move through their lifecycle in `jobs.rs` (publish ~30 min before the
  event, archive at its end); the package job acts as the package's stored actor, rebuilt at run time
  (#607).

## Verifying a change

**Verifying any change here is the `trajectory-qa` skill.** Load it before reporting a feed,
trajectory, metering, flow or TMI change as done. At minimum:

- Every caller above has a test that fails when the change is reverted at that site.
- A behavior change states its measured effect in the PR: the ETA shift at named distances, the
  spacing at a named fix, before and after (#361's commit message is the model).
- A run of the real stack against the live feed, on a throwaway database, with the surfaces that
  moved compared side by side (`test-quality.md` → "jsdom vs the running stack — the decision rule").
- Tests never call the real VATSIM, VATUSA, AWC or winds APIs; build the `Snapshot` and tables in the
  test.
