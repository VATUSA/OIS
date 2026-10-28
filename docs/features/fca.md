# FCA logic, end to end

How an FCA decides which flights it includes, how it orders and spaces them, what each write
changes, and what each read and screen shows. Each area ends with what was checked and what was
found. Written for #838 against `next` at `333a7831`. Tables, permissions, and the route list are in
[flow.md](flow.md). This doc does not repeat them.

Out of scope here, with links: ETA computation itself (`feed/predict.rs`, `feed/trajectory.rs`;
design doc planned by #814 at `docs/architecture/prediction.md`), the release-time formula (#815),
IDST Set (#816), the scheduler re-check and freeze horizon (#817), manual-order late joiners
(#819), naming (#820), profiles (#818, #828), winds (#836), and IDST duplicate rows (#834).

Paths below are relative to `backend/src/` unless they start with `web/` or `backend/migrations/`.

```
VATSIM snapshot (handlers/flow.rs::feed_view :61)
  └─ pilots with a flight plan ─▶ fca::route_path(live lat/lon/hdg/gs)
     (prefiles are not candidates, #834)              ▼
        fca_crossing_for (flow.rs:202): excluded? ─▶ filters ─▶ fca::crosses ─▶ scope
                                                      ▼  crossing + along_nm
        predict::trajectory ─▶ eta, crossing groundspeed  (one entry point: #813)
        flow.fca_release (by callsign) ─▶ frozen_ms
                                                      ▼  MeterInput[]
        finalize (flow.rs:2160) ─▶ manual_order (:2082) ─▶ fca::meter (fca.rs:348)
                                                      ▼
        fca_traffic · flight_advisory · list_idst · mark_release · clear_release
                                                      ▼
        web/src/pages/fca (detail, ladder, overview-panel) · web/src/pages/idst

writes: create/update/delete/reorder ─▶ flow.fca ─▶ topic flow.fca
        mark/clear/swap               ─▶ flow.fca_release ─▶ topic flow.release ─▶ back into meter
```

## 1. Geometry and crossing

**Route to a path.** `fca::route_path` (`feed/fca.rs:209`) resolves the filed route to anchors
(`route_anchors`, `feed/fca.rs:32`) and returns `None` below 2 anchors (`feed/fca.rs:223`). It then
trims the part already flown, on one of two branches chosen by `gs >= 50` (`feed/fca.rs:226`):

- **Airborne**, `remaining_anchors` (`feed/fca.rs:107`). `project_forward_index`
  (`feed/fca.rs:71`) finds the nearest leg by perpendicular distance and moves one anchor ahead
  when the aircraft is within 3 nm of the leg end (`feed/fca.rs:99`). Fixes more than 5 nm away and
  more than 110° off the current heading are then skipped (`feed/fca.rs:113-121`). The current
  position is prepended.
- **Ground**, `forward_route_from_position` (`feed/fca.rs:147`), which uses position only, never
  heading. At the departure field the airport anchor is replaced by the aircraft's position. At the
  arrival field the path collapses to one point, so `route_path` returns `None` and a landed
  aircraft is in no FCA (`feed/fca.rs:232`, #213).

**Crossing.** `fca::crosses` (`feed/fca.rs:293`) runs `path_crossing` (`feed/fca.rs:168`). Each
route leg is densified into great-circle chords about `DENSIFY_STEP_NM` = 40 nm long
(`feed/fca.rs:10`, `densify` at `feed/fca.rs:504`). Each chord is intersected with each FCA segment
in one flat frame centered on the path's first point (`local`, `feed/fca.rs:468`, using
`NM_PER_DEG` = 60 at `feed/fca.rs:9`). Parallel segments never intersect (`denom.abs() < 1e-9`,
`feed/fca.rs:490`). The crossing kept is the one with the smallest distance along the route
(`feed/fca.rs:188`). FCA segments are not densified.

`fca::crossing_for` (`feed/fca.rs:299`) is `route_path` plus `crosses`. Only `feed/fca.rs`'s tests
call it. Production code resolves the path once per aircraft and passes it to the handler's
decision below.

**The inclusion decision.** `fca_crossing_for` (`handlers/flow.rs:202`) is the one rule every FCA
surface uses (#360). In order:

1. **Manual exclusion** for the FCA's ARTCC: `is_manually_excluded(…, Some(&fca.artcc), …)`
   (`handlers/flow.rs:130`, called at `:211`). The set comes from `AppState::flight_exclusions`.
   Rows expire after `EXCLUSION_TTL_HOURS` = 2 (`handlers/flight_exclusions.rs:34`).
2. **Filters**, `passes_filters` (`handlers/flow.rs:167`):
   - destinations and origins match with or without a leading `K` (`airport_match`, `:92`);
   - fixes match a route token after stripping `/…` and trailing digits (`route_has_fix`, `:98`);
   - the altitude band (`altitude_matches`, `:224`) matches when **either** the filed altitude or
     the current altitude is in band. No band always matches, and unknown on both does not exclude.
     The filed altitude is parsed by `filed_altitude_ft` (`:152`): all digits concatenated, values
     ≤ 600 read as flight levels, and 0 or no digits read as unknown.
3. **Geometry**: `fca::crosses` (`handlers/flow.rs:217`).
4. **ARTCC scope** on the crossing point, `passes_scope` (`handlers/flow.rs:109`). An empty scope,
   or boundary data that failed to load, means no restriction (`:110`).

Callers: `fca_counts` (`handlers/flow.rs:712`), `build_flight_advisory` (`:1085`), and
`build_candidates`, whose prefile loop (`:1990`) #834 removed, leaving the pilot loop (`:1871`).

`FcaBody.dir` (any/N/S/E/W) is stored (`repos/flow.rs:154`) and read nowhere in the backend.

**Tests.** `feed/fca.rs`: `detects_a_route_crossing_a_line` (`:586`),
`no_crossing_when_line_is_off_route` (`:603`), the three ground-trim tests (`:612`, `:627`, `:641`),
`loads_bundled_nav_and_matches_a_real_route` (`:660`), the three `route_path_named_*` tests (`:698`,
`:763`, `:829`), `airborne_past_the_line_does_not_match` (`:1174`),
`dogleg_route_crosses_even_when_heading_away` (`:1185`). `handlers/flow.rs`: `mod
filed_altitude_tests` (`:2864`), `mod manual_exclusion_tests` (`:4435`), `mod fca_inclusion_tests`
(`:4496`).

The heading skip in `remaining_anchors` is pinned by `airborne_past_the_line_does_not_match`
(`feed/fca.rs:1174`): without the skip, that test's path crosses the line.

**Untested branches.** The 3 nm advance. A route that crosses the line twice. Any FCA with more than
2 points. The origin and fix filters (only destinations are tested). The `airspace.is_empty()`
short-circuit. `dir`.

### Findings, area 1

- **A scope code with no boundary polygon excludes every flight, silently.** `Boundaries::contains`
  returns false for an unknown code (`feed/airspace.rs:43-46`). The same file notes that ZAK and ZSU
  have no polygons (`feed/airspace.rs:34-36`). `scope` is written without validation
  (`repos/flow.rs:151`). An FCA scoped to one of them, or to a typo, shows no traffic and logs
  nothing. Filed as #849.
- **The altitude band and the ETA parse the filed altitude differently.** `filed_altitude_ft` reads
  `"VFR"` as unknown, so the flight passes the band. `trajectory::parse_alt_ft`
  (`feed/trajectory.rs:19-29`) reads the same string as 35,000 ft, and `build_candidates` times the
  flight at that cruise altitude (`handlers/flow.rs:1884`; `:1996` was the prefile loop #834 removed). Both parsers concatenate every
  digit, so `"FL350/370"` reads as 350,370 ft, which is out of every band. Added to #813.
- **A ground aircraft's field elevation counts as its current altitude in the band.** Live pilots
  pass `Some(p.altitude)` whatever their phase (`handlers/flow.rs:1877`), and only `0` is dropped
  (`:179`). An FCA whose band reaches down to a field's elevation (for example `max_fl` 100) takes
  in every departure parked at that field, whatever it filed. Whether low bands are used in practice
  is not verified.
- **`dir` is a dead field.** It is stored and returned, and it changes nothing.
- *Checked, not filed.* The heading skip (`feed/fca.rs:113-121`) does not contradict the
  "no heading gate" note on `crosses` (`feed/fca.rs:287-292`). That note is about the crossing
  point, and the dogleg case it cites keeps its fix, because the fix is ahead of the nose. The skip
  can drop the fix of an aircraft outbound in a hold. The path then runs straight to the next fix,
  and a line drawn across the skipped leg is missed. That is a narrow case, and no report shows it.
- *Checked, not filed.* `local` and `unlocal` (`feed/fca.rs:468-478`) are affine, so the frame
  itself moves neither inclusion nor `along_nm`. The approximation is that each FCA segment is a
  straight line in latitude and longitude, not a great circle, because FCA segments are not
  densified. On a long diagonal segment the crossing point sits a little off the drawn line.

## 2. Candidates

`build_candidates` (`handlers/flow.rs:1829`) turns a snapshot into `FcaFlight` rows plus a parallel
`MeterInput` list. It builds no schedule.

**Live pilots** (`build_candidates`, `handlers/flow.rs`):

1. No flight plan: skipped.
2. `predict::trajectory(inp, Flight::Pilot(p), None, now)` (#813) resolves the route from the live
   position, heading and groundspeed, and decides airborne (`gs >= 50`), the direct-route padding
   and the departure runway from `inp.ground.assigned`, the map `list_idst` reads. No route and no
   known destination: skipped.
3. `fca_crossing_for` with the current altitude, on `traj.path()`. Skipped before either push,
   because the two lists are consumed as parallel slices.
4. The trajectory builds its profile, wind and ground allowance only now, for a flight that
   crosses.
5. `traj.eta_at(cross.along_nm)`, the same clock airport flow, sector demand and the per-fix table
   read.
6. `cross_speed = traj.gs_at(cross.along_nm).max(MIN_CROSS_SPEED_KT)` (120 kt), the groundspeed
   predicted at the line. MIT spacing uses it (#355).
7. `frozen_ms` is the release's CTA when `flow.fca_release` holds a row for this callsign.

**Prefiles** are not candidates (#834). Until #834 a second loop added every prefile as a
`proposed` row, so a prefile that lingered after its pilot connected listed the aircraft twice, and
an unconnected prefile took a slot that connected departures metered behind.

**Release map.** `load_releases` (`handlers/flow.rs:2191`) reads every row for the FCA by callsign
(`repos/flow.rs:525-535`). There is no time filter.

**`finalize`** (`handlers/flow.rs:2160`) runs `manual_order` (`:2082`) and `fca::meter`, then sets
`cross_time`, `delay_sec`, `delay_min = (delay_sec + 30) / 60` (`:2169`), and
`delay_nm = delay_sec / 3600 × cross_speed` (`:2173`), and sorts by `seq`.

**Callers of `build_candidates`.** `metered_flights` (serves `fca_traffic` and
`build_flight_advisory`), `list_idst`, `mark_release` and `clear_release` all build their inputs
with `TimingTables::live`, so each reads the stored departure runways
(`flow.departure_runway_assignment`) once per request (#813).

**Tests.** `mod prefile_skip_integration_tests` and `mod mit_cross_speed_wiring_tests` in
`handlers/flow.rs`, the latter covering an airborne and a ground pilot, and
`handlers/flow_prefile_candidate_tests.rs` through the router (#834). `mod fca_inclusion_tests`
(`:4496`) pins the board to the shared decision.

**Untested branches.** The pinned time a release gives: router tests send a non-empty release map
and assert `released`, but none checks `cross_time`, so `frozen_ms` (set in `build_candidates`
and `build_candidates_with`) could be dropped with the suite green. A pilot with no flight plan. `finalize`'s rounding and `delay_nm`. `manual_order` through `finalize`.

### Findings, area 2

- **IDST and every other caller meter the same FCA with different runway inputs.** The table above
  shows the split. The doc comment on `assigned` (`handlers/flow.rs:1842-1845`) calls it "the same
  map `list_idst` reads", but only `list_idst` passes it. A departure with an assigned runway gets
  one ETA on IDST and another on the FCA page and from RDY. **Owned by #813** (its input 5).
- **The same callsign as a pilot and a prefile gave two candidates.** Fixed by #834, which stops
  treating any prefile as a candidate. A departure crossing two FCAs still has a row in each (#862).
- **A disabled FCA still meters and still takes releases.** `fca_counts`, `build_flight_advisory`
  and `list_idst` skip a disabled FCA (`handlers/flow.rs:683`, `:1080`, `:2390`). `metered_flights`
  checks only the point count (`:2212`), and `mark_release` checks no flag at all. A disabled FCA
  therefore shows a full board next to a badge of 0 and accepts RDY and SET. Its releases do not
  count in `locked_wheels_up` (`repos/flow.rs:556`). Filed as #846.
- **The flight advisory ignores prefiles.** `build_flight_advisory` looks only at the
  snapshot's pilots (`handlers/flow.rs:951-955`) and answers `found: false` otherwise. Since #834
  the board does not meter prefiles either.
- *Checked, not filed.* `gs >= 50` counts a takeoff roll as airborne. The threshold is one of the
  disagreeing inputs **#813** already lists (its input 3).

## 3. Metering

`fca::meter` (`feed/fca.rs:348`) takes the `MeterInput` list, the FCA's mode, rate and MIT, and
an optional manual order.

**Separation** (`feed/fca.rs:355-364`):

- `mode == "mit"`: `mit / max(cross_speed, 60) × 3600` seconds, at the placed aircraft's own
  crossing speed.
- otherwise, if `rate > 0`: `3600 / rate` seconds.
- otherwise 0, which means no spacing at all. Writes clamp rate to 0-240 (default 30) and MIT to
  0-200 (default 15) (`repos/flow.rs:156-157`), so 0 is a legal value that turns metering off.

The handler holds a second copy of this formula, `separation_ms` (`handlers/flow.rs:2116`), which
RDY uses.

**Auto mode** (`feed/fca.rs:402-432`). Pinned flights (airborne, or holding a release) sort ahead
of the rest, then by ETA (`:408-414`). Each in turn is placed at its frozen time if it has one,
else at `earliest_slot(eta, committed, sep)` (`:418-422`), and is added to `committed`. Sequence
numbers follow the scheduled time (`:427-431`).

**`earliest_slot`** (`feed/fca.rs:444`): from the ETA, move to `c + sep` for any committed `c`
closer than `sep`, and repeat until nothing moves. One `sep` is used in both directions.

**Manual mode** (`feed/fca.rs:370-401`). The stored order comes first and every other flight is
appended by ETA (`:372-376`). Each flight is chained `max(eta, prev + sep)` behind the one before,
except a frozen flight, which keeps its time and becomes the new `prev` (`:385-393`). Sequence
numbers are the order itself (`:399-401`).

`delay_sec = max(sched - eta, 0) / 1000` (`feed/fca.rs:437`).

**Releases enter the sequence** as `frozen_ms` (area 2). `mark_release` computes the CTA:

- **RDY**, `rdy_slot` (`handlers/flow.rs:2144`): in manual mode the ladder's slot plus
  `RELEASE_MARGIN_MS` = 15 s (`:508`). In auto mode, `earliest_slot` against every other pinned
  flight at its **metered** time (`committed_crossings`, `:2099`, #356), with the margin added to
  the separation.
- **SET**: the typed time plus the airborne ETE, `eta - now - ground_allowance`
  (`handlers/flow.rs:2630`, #815). It runs no slot search; that is **owned by #816**.
- The stored EDCT is `release_ms`: the CTA less the airborne ETE, `cta - (eta - now - ground_allowance)`
  (`handlers/flow.rs:2133`, `:2635`, #815), so RDY stores a wheels-up and SET stores the typed time.

**Tests.** `feed/fca.rs:882-1141`: MIT at the follower's speed, rate metering, airborne priority,
a frozen ground release, two conflicting airborne crossings, and four manual-order cases.
`mod release_spacing_tests` (`handlers/flow.rs:3817`) covers `rdy_slot` and `separation_ms`.

**Untested branches.** Airborne against frozen in auto mode. A frozen flight whose ETA is past its
frozen time. Duplicate or out-of-range indices in the order. MIT with a slow leader and a fast
follower. Rate or MIT 0 through `meter`. Equal-ETA ties.

### Findings, area 3

- **Frozen times are never checked against their neighbors.** In manual mode a frozen flight can sit
  inside the gap, or before the flight ordered ahead of it, and the flight after it is chained from
  that frozen time. `manual_order_chains_airborne_and_pins_frozen` (`feed/fca.rs:1055`) asserts this
  as intended (noted on #817). In auto mode, pinned flights sort by ETA, not by frozen time, so an
  airborne flight can be slotted inside a release's gap before the release is committed. **Owned
  by #817** (its cases 2 and 3).
- **A frozen flight that is now late shows as on time.** `delay_sec` clamps to 0. **Owned by #817**
  (case 1).
- **MIT spacing uses the placed aircraft's speed even when it ends up in front.** **Owned by #817**
  (case 4).
- **A manual order is stored verbatim, duplicates included.** `ReorderRequest.order`
  (`models/mod.rs:2717`) is saved as sent (`repos/flow.rs:435`), and `manual_order` matches it by
  exact string (`handlers/flow.rs:2086`). A repeated callsign is scheduled at every occurrence, and its
  last one wins: it is re-chained behind whatever precedes that occurrence, and the rank of its
  earlier occurrence is left empty in the sequence numbers. A lowercase entry
  matches nothing, and the flight falls to the tail. The tail append itself is **#819**, and this was
  added there.
- *Checked, not filed.* The separation formula exists twice (`feed/fca.rs:355-364`,
  `handlers/flow.rs:2116-2124`). The two copies agree today, and #817 (case 4) changes both.

## 4. Writes

| Route | Handler | Gate | Writes | Topic |
| --- | --- | --- | --- | --- |
| `POST /api/v1/flow/fcas` | `create_fca` (`handlers/flow.rs:361`) | `flow.fca.update` + ARTCC (`require_fca_write_scope`, `:252`) | `flow.fca` | `flow.fca` (`:372`) |
| `PUT /api/v1/flow/fcas/{id}` | `update_fca` (`:388`) | `flow.fca.update` + event-planner rule (`:277`, #736) + old and new ARTCC | `flow.fca` | `flow.fca` (`:414`) |
| `DELETE /api/v1/flow/fcas/{id}` | `delete_fca` (`:429`) | `flow.fca.delete` + event-planner rule + ARTCC | `deleted_at = now()` (`repos/flow.rs:318`) | `flow.fca` (`:444`) |
| `PUT /api/v1/flow/fcas/{id}/order` | `reorder_fca` (`:2840`) | `flow.fca.update` + hidden-event 404 (`:311`, #746) + ARTCC | `manual_order`, `manual_seq` | `flow.fca` (`:2859`) |
| `POST /api/v1/flow/fcas/{id}/release/{callsign}` | `mark_release` (`:2536`) | `flow.fca.update` + hidden-event 404 + ARTCC (`:488`) + release authority (#585) | upsert `flow.fca_release` | `flow.release` (`:2609`) |
| `DELETE /api/v1/flow/fcas/{id}/release/{callsign}` | `clear_release` (`:2639`) | same | delete `flow.fca_release` | `flow.release` (`:2672`) |
| `POST /api/v1/flow/fcas/{id}/swap` | `swap_releases` (`:2741`) | same, plus same airport and runway (`:2791`) | swap two rows | `flow.release` (`:2782`) |
| `POST`/`DELETE /api/v1/flow/fcas/{id}/exclusions/{callsign}` | `handlers/flight_exclusions.rs:145`, `:182` | `flow.fca.update` + hidden-event 404 + ARTCC | `flow.manual_flight_exclusion` | `flow.fca` (`:172`, `:198`) |

Event FCAs are written through `/api/v1/events/{id}/fcas/...` (`handlers/events.rs:1611-1769`),
gated on `events.plan.read` for the list and `events.plan.update` for every write. Create, update,
delete and the auto-publish toggle publish no topic. Publish and archive publish `flow.fca`
(`handlers/events.rs:1717`, `:1743`). The `event_fca_lifecycle` job (`jobs.rs:1501`, every
`EVENT_FCA_INTERVAL` = 60 s at `jobs.rs:62`)
publishes planned FCAs flagged `auto_publish` 30 minutes before their event starts, archives live
ones at its end (`repos/flow.rs:295-316`), and publishes `flow.fca` only when a row changed
(`jobs.rs:1511-1513`).

**What an FCA write does to releases.** None of these writes touches `flow.fca_release`:

- An edit to the line, rate or MIT leaves every release pinned at its old CTA, and the manual order
  stays. A flight that no longer crosses keeps its row, and that row is invisible on the board.
- Disabling keeps the rows, and they come back pinned on re-enable.
- Delete is soft (`repos/flow.rs:318-327`), so the `on delete cascade` in
  `backend/migrations/0023_flow_fca_release.sql:8` never fires.
- Archiving keeps the rows.

**Tests.** `mod fca_scope_tests` (`handlers/flow.rs:5303`) for the plain routes;
`handlers/flow_event_fca_visibility_tests.rs` for the #746 routes; `machine_actor_tests.rs` for
release authority, scope and provenance; `mod release_swap_tests` (`handlers/flow.rs:4778`);
`mod scope_tests` in `handlers/flight_exclusions.rs:206`.

**Untested branches.** `publish_event_fca`, `archive_event_fca`, `set_event_fca_auto`,
`delete_event_fca`, and `run_event_fca_lifecycle`. Writes to a soft-deleted FCA. What
`update_fca` does to releases and the manual order.

### Findings, area 4

- **The event FCA writes check no ARTCC scope.** All six event routes take
  `RequirePermission<EventsPlanUpdate>`, which passes when the caller holds the permission at any
  facility, and `owned_event_fca` (`handlers/events.rs:1589`) checks only the event id. The plain
  routes check the stored and new ARTCC (`handlers/flow.rs:404-410`, #636). A planner scoped to one
  facility can therefore edit, delete or publish another facility's event FCA. Filed as #854.
- **`flow.fca_release` is never pruned, and the release map is keyed by callsign.** The only
  `DELETE` is `delete_release` (`repos/flow.rs:686`), and the cascade needs a hard delete that never
  happens. The comment at `jobs.rs:106-109` says these rows "vanish with their initiative", which
  is not true for FCAs. A callsign that flies the same FCA on a later day, as airline callsigns do,
  is pinned to the old CTA (`handlers/flow.rs:1928-1933`). It shows as released with yesterday's
  EDCT, and that EDCT also reaches `locked_wheels_up`. Filed as #844.
- **Releases on planned or archived event FCAs still lock a departure.** `locked_wheels_up`
  (`repos/flow.rs:543-560`) joins on `f.enabled and f.deleted_at is null` and ignores
  `event_status`. Planners may release on hidden FCAs (#746 owner decision), and archiving keeps
  the rows, so those times feed sector demand (`handlers/sector_demand.rs:209`) and the departures
  list (`handlers/feed.rs:562`). Every other surface hides those FCAs (`repos/flow.rs:20-22`).
  Filed as #845.
- **Editing or deleting an event FCA publishes no realtime topic.** `update_event_fca` and
  `delete_event_fca` (`handlers/events.rs:1658`, `:1684`) skip `state.publish`, unlike the plain
  routes. A published event FCA edited this way changes live metering, and other screens see it only
  on their next poll. Both also discard the repo's "changed" result and answer 200 for a no-op.
  Filed as #847.
- **Writes go through on a soft-deleted FCA.** `get_fca` has no `deleted_at` filter
  (`repos/flow.rs:70-76`), and no write path checks it. Edit, reorder, RDY, clear, swap and exclude
  on a deleted id succeed and publish. Filed as #848.
- **RDY, SET and clear run the metering pass on an async worker.** `mark_release` and
  `clear_release` call `build_candidates` inline (`handlers/flow.rs:2563-2582`, `:2679-2696`).
  `metered_flights`, `list_idst` and `fca_counts` run the same work in `spawn_blocking`
  (`:2227`, `:2419`, `:679`), and `metered_flights`'s comment (`:2199-2204`) records that the
  inline form can starve the runtime. Filed as #855.
- **The exclusion gate does not normalize an empty ARTCC** the way the FCA write gate does
  (`handlers/flight_exclusions.rs:110` against `handlers/flow.rs:261`). For an FCA with no ARTCC, a
  national holder with an exception list passes one gate and fails the other.
- *Checked, not filed.* Keeping releases and the manual order across an edit is a decision, not a
  defect. Whether an edit should re-meter released flights belongs with #817's re-check.

## 5. Reads

| Read | Line | Gate | What it computes |
| --- | --- | --- | --- |
| `GET /api/v1/flow/fcas` | `handlers/flow.rs:348` | public (#586) | `list_fcas`: not deleted, not an unpublished event FCA (`repos/flow.rs:24`) |
| `GET /api/v1/flow/counts` | `handlers/flow.rs:662` | public | per enabled FCA, connected pilots through `fca_crossing_for` (no prefiles since #834). No ETA, no metering |
| `GET /api/v1/flow/fcas/{id}/traffic` | `handlers/flow.rs:2276` | public for listed FCAs; unpublished event FCA needs `events.plan.read`; a deleted one needs a sign-in | `metered_flights`: full board, with releases and their versions |
| `GET /api/v1/flow/idst` | `handlers/flow.rs:2345` | `flow.fca.read` | every enabled FCA metered with the runway map, then ground rows at airports in scope (`:2442-2445`), with an advisory EDCT for unreleased rows (`:2450-2457`) |
| `GET /api/v1/stats/hist/fcas` | `handlers/stats.rs:871` | `stats.data.read` | `list_fcas_at` (`repos/flow.rs:35`): FCA definitions only, no crossings |

All of them share one inclusion rule. They do not share one ETA: see the runway map in area 2.

**Visibility.** Traffic admits an unpublished event FCA on `events.plan.read`
(`handlers/flow.rs:2301`). The live operations admit on `events.plan.update`
(`is_event_planner`, `:295`). Counts and IDST never show an unpublished event FCA, even to a
planner, because both read `list_fcas`. This matches #586 and #746 as built.

**Tests.** `mod fca_traffic_visibility_tests` (`handlers/flow.rs:5939`). IDST has router tests
for provenance (`handlers/machine_actor_tests.rs:1060`, `:1103`, `:1611`) and for its advisory
EDCT: `idst_proposes_the_same_edct_a_release_stores` and
`idst_gives_a_rolling_pilot_its_own_runway_tier_allowance`. The `fca_counts`
handler is covered through the router by `handlers/flow_prefile_candidate_tests.rs` (#834). No test
covers `hist_fcas`.

### Findings, area 5

- **The badge and the board disagree.** The count included prefiles, and the detail panel hid
  `proposed` rows (`web/src/pages/fca/detail.tsx:221`); #834 removed prefiles from both the count and
  the board. Counts still skip a disabled FCA while traffic serves it (area 2), so the comment at
  `handlers/flow.rs:710-711` ("a badge count can never disagree with the board") holds for an enabled
  FCA only. Filed as #853; the disabled-FCA gap is #846.
- **Historical FCAs are filtered by their state now.** `list_fcas_at` applies the current
  `event_status` and the current `enabled` flag (`repos/flow.rs:33-38`). An event FCA that was
  live during the replayed window and is archived now is missing from the replay. One published
  now appears before it was published. Filed as #852, with the web half below.
- IDST rows for one aircraft under two FCAs: **owned by #862** (path 2 of #834).

## 6. Web

`web/src/pages/fca/index.tsx` renders `FcaMapView` (`web/src/components/map/FcaMapView.tsx`), which
loads the selected FCA's traffic (`useFcaTraffic`, `web/src/lib/fca.ts:407`, key
`["fca-traffic", id, debug]` at `:414`) and the counts (`useFcaCounts`, `:342`).

- **`detail.tsx`** lists non-`proposed` rows (`:221`), shows Air, Ground and summed delay
  (`:242-247`), and gives each strip its sequence, crossing time, delay, distance, `delay_nm` and
  release. RDY and SET are hidden for airborne flights (`:59`). Dragging sends the new order of the
  listed callsigns (`:249-257`).
- **`ladder.tsx`** drops `proposed` rows and rows without a crossing time, and places each strip at
  `cross_time − now` (`web/src/pages/fca/ladder.tsx:27-47`).
- **`overview-panel.tsx`** is read-only and also drops `proposed` rows
  (`web/src/pages/fca/overview-panel.tsx:164`).

The page recomputes no crossing time or ETA. It only positions strips by time and renumbers them
during an optimistic reorder (`web/src/lib/fca.ts:598-611`). Realtime: `flow.release`
invalidates the traffic, IDST and departures queries by prefix (`web/src/lib/realtime.ts:46`), and
`flow.fca` invalidates the FCA list, traffic, counts, IDST and event FCAs (`:48`).

**Tests.** `web/src/pages/fca/detail.dom.test.tsx` (#746 controls),
`web/src/pages/fca/index.dom.test.tsx`, `web/src/pages/fca/ladder.test.ts`,
`web/src/lib/fca.dom.test.tsx` (the release, clear and reorder hooks), and
`web/src/lib/realtime.dom.test.ts:166-169` (the two topics' keys).
`web/src/components/map/FcaMapView.artcc.dom.test.tsx` mocks the overview panel. Nothing covers
`overview-panel.tsx` or historical mode. `fca.dom.test.tsx` seeds and asserts the two-element
key `["fca-traffic", ID]` (`web/src/lib/fca.dom.test.tsx:44`, `:59`, `:123`), so it passes with the
defect below and has to change with its fix.

### Findings, area 6

- **The release and reorder mutations write a cache entry the detail panel never reads.**
  `useMarkRelease`, `useClearRelease` and `useReorderFca` call `setQueryData(["fca-traffic", fcaId])`
  (`web/src/lib/fca.ts:506`, `:531`, `:586`). The panel's key carries a third element, `debug`
  (`:414`), and `setQueryData` matches exactly. The two-element key belongs to the ARTCC overview's
  `useFcaTrafficMany` (`:440`), whose comment at `:434-435` wrongly says it shares the panel's key.
  So the release writes land in the overview's cache, and the reorder updates nothing unless the
  overview has that FCA cached (`:600`). In the panel, a dragged strip snaps back until the server
  answers and the prefix invalidation refetches. A fix that only adds `debug` to the mutations'
  key would break the overview, so it has to update both keys. Filed as #850.
- **Release and reorder controls ignore the FCA's ARTCC.** `canOperate`
  (`web/src/pages/fca/detail.tsx:237`) checks that the user holds `flow.fca.update` somewhere and
  that the FCA is live or the user is a planner, but never the FCA's ARTCC. Exclusions already ask
  the server (`:229-232`). A controller viewing another facility's FCA is offered RDY, SET, clear
  and drag, and gets a 403. Filed as #851.
- **Historical mode mixes a past FCA list with live counts and traffic.** Inside a
  `HistoricalProvider`, `useFcas` reads the instant (`web/src/lib/fca.ts:158`) and switches to
  `/stats/hist/fcas` (`:169-175`). `useFcaCounts` and `useFcaTraffic` ignore it. The dashboard FCA
  widget joins the two (`web/src/features/dashboard/sources.ts:325-333`). Part of #852.
- Prefiles left out of a manual order fall behind everyone: **owned by #819** (its item 2). Since
  #834 a prefile is never a candidate, so it is never in the order.

## Determination for #812 (recommendation, for the operator to confirm)

The audit confirms each sub-issue's premise in code. It found nothing that removes the need for
any of them, and three changes of scope.

| Issue | Recommendation | Basis |
| --- | --- | --- |
| #812 epic | Go ahead as written | Each defect it lists is present on `next`. The new findings above sit beside the epic and do not change its order. |
| #815 EDCT read as wheels-up | Go ahead (PR #829 is in review) | `handlers/flow.rs:2600` still subtracts the full ground ETA on `next`. |
| #813 single prediction entry point | Go ahead, with one change: add the filed-altitude parser split (area 1) to its list of disagreeing inputs | Input 5 (runway map) is confirmed at `handlers/flow.rs:2241`, `:2437`, `:2577`, `:2693`. |
| #816 IDST Set | Go ahead as written | `handlers/flow.rs:2596` pins the typed time with no slot search. |
| #814 prediction design | Go ahead as written | Out of this audit's reach beyond the link above. |
| #817 scheduler re-check | Go ahead, with one change: update the test that pins case 3 | Cases 1-4 are confirmed at `feed/fca.rs:385-393`, `:410-424`, `:437`, `:454`. It should also update `manual_order_chains_airborne_and_pins_frozen` (`feed/fca.rs:1055`), which pins today's behavior as correct. |
| #819 manual order | Go ahead, with one change: reject or dedupe repeated and wrong-case callsigns in the stored order (area 3) | It already owns the tail append and prefiles in the order. |
| #828 profile builder | Go ahead as written | Not touched by this audit. |
| #818 per-type profiles | Go ahead as written | Not touched by this audit. |
| #820 naming | Go ahead as written | Not touched by this audit. |
| #834 IDST duplicates | Go ahead as written | Path 1 is confirmed: neither loop of `build_candidates` checks the other. |
| #836 winds | Go ahead as written | Not touched by this audit. |

Findings filed from this audit: #844, #845, #846, #847, #848, #849, #850, #851, #852, #853,
#854, #855.
Added to existing issues: #813 (filed-altitude parsers), #817 (the test that pins case 3), and
#819 (duplicate and wrong-case callsigns in the stored order). Four findings are recorded here
only, by the operator's decision: the field-elevation band case and `dir` (area 1), the advisory
ignoring prefiles (area 2), and the exclusion gate's empty ARTCC (area 4). The untested branches
listed in each area are recorded here and not filed, also by the operator's decision.
