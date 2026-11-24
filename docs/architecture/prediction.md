# The prediction model

OIS predicts when a flight reaches a point: an FCA line, a fix, its arrival field. One model does
it, `feed/trajectory.rs`, behind one entry point, `feed::predict::trajectory`, which every surface
calls (#813). The callers and the command that lists them are in
[`docs/features/flow.md` → The ETA model and its callers](../features/flow.md#the-eta-model-and-its-callers).
This page says what the model is: what it reads, how it flies a flight, what a release time means,
and where it differs from FAA TBFM and from vTBFM. The model is a port of vTBFM's (#985);
[`trajectory-special-cases.md`](trajectory-special-cases.md) lists the vTBFM rules OIS has not taken.

Constants are cited as `NAME` (`path:line`), the line that defines them, paths relative to the
repository root. `docs_tests::the_prediction_doc_cites_each_constant_where_it_is_defined` fails
when a cited line no longer defines its constant.

## What the model reads

For each flight:

- **Its position**, for a connected pilot: latitude, longitude, heading, groundspeed and altitude
  from the VATSIM feed, polled `REFRESH_BUFFER_SECS` (`backend/src/feed/mod.rs:61`) seconds after
  each publish's `update_timestamp`, retried every `FAST_POLL_SECS` (`backend/src/feed/mod.rs:65`)
  seconds once that time has passed, and every `STALE_POLL_BACKOFF_SECS`
  (`backend/src/feed/mod.rs:73`) while the feed repeats itself (`next_poll_delay`). A
  prefile has no position. It is timed only when it holds a lock, from its departure field's
  reference point; one with no lock gets no proposed wheels-up and is not timed (#814).
- **Its flight plan**: departure, arrival, route, filed cruise altitude, filed cruise speed,
  aircraft type and filed departure time (`deptime`).
- **A released wheels-up**, when it holds one: a ready time typed into SET, an FCA release, an
  issued CFR or a published GDP EDCT.

And four tables the feed caches, so the model never queries the database:

- the navigation data, which resolves the filed route to a path (`fca::route_path`);
- the airports, for positions and field elevations;
- the winds aloft (below);
- the aircraft performance: ERAM's tables for 2,692 types, bundled in
  `backend/data/aircraft-performance.json`, and the rows of `flow.aircraft_profile`. A flight flies its
  type's row when there is one, else its type's ERAM record, else its engine family's row, its wake
  class's row, or the default row (`ProfileTable::resolve`). The FCA debug view names the one flown:
  `type:B738`, `eram:B738`, `family:jet`, `wake:H` or `default`. Until the rows have loaded from the
  database, ERAM's tables and a built-in default row fly (`ProfileTable::built_in`).

The entry point decides each input once:

- **Airborne** means a groundspeed of at least `AIRBORNE_GS_KT` (`backend/src/feed/predict.rs:277`)
  knots, at any altitude.
- **The route** is the filed route resolved from where the flight is. When it does not resolve,
  the model flies the great circle to the arrival field. A flight not yet airborne on a route that
  resolves to its two endpoints only is padded by `GROUND_ROUTE_FACTOR`
  (`backend/src/feed/predict.rs:43`), since no airway routing flies the direct line. A ground or
  prefiled flight with no path to fly, a prefile from an unknown field or a route that does not
  resolve to an unknown arrival field, is timed over `NOMINAL_ROUTE_NM`
  (`backend/src/feed/predict.rs:289`) nautical miles. A connected ground pilot within `ARRIVED_NM`
  (`backend/src/feed/predict.rs:479`) of an arrival field it did not depart from has arrived: it
  gets no great circle to that field, and airport flow labels it arrived.
- **The filed altitude** is the first run of digits in the filing, so `FL350/370` is 35,000 ft; an
  unreadable filing is timed at 35,000 ft (`trajectory::parse_alt_ft`).
- **The filed cruise speed** is believed from 60 to `MAX_PLAUSIBLE_TAS`
  (`backend/src/feed/trajectory.rs:17`) knots. A type on an ERAM record cruises at it, held inside the
  record's minimum and maximum true airspeed at the altitude it cruises at, and at the middle of that range
  when the filing is not believed. A type on a row has it capped by the row's cruise speed, which also
  stands in for a filing that is not believed; a row with no cruise speed falls back to an altitude
  default (`trajectory::capped_cruise_tas`).
- **The departure** is the filed airport's position, for the straight-line distance a flight has come
  from it. A departure the airport table does not hold has no distance.

## Wheels-up

Every trajectory starts at wheels-up and flies from the runway. No taxi time is added anywhere
(#814): taxi belongs to the tower, which hands the model a runway time.

- An airborne flight is wheels-up now.
- A ground flight held to a wheels-up is wheels-up then, or now if that time has passed. The hold is
  the latest of its issued CFR, its releases in live FCAs (RDY and SET each store one) and its
  published GDP slot (`repos::flow::locked_wheels_up`), which the FCA board, airport flow and the
  departures list read for every flight (`TimingTables::live`) and the per-fix table for its one
  flight (`TimingTables::live_for`). The GDP draft
  reads every lock but its own airport's GDP slots, so a revise or compress can still move a flight
  earlier (`repos::flow::locked_wheels_up_outside_gdps_at`). Such a flight's ETA runs from its
  lock, so its delay column shows only delay added after the lock; the delay it took on the ground
  is in its release time.
- A connected ground pilot with no scheduler time is wheels-up at its proposed time: its filed
  departure time plus `PROPOSED_WHEELS_UP_MIN` (`backend/src/feed/predict.rs:37`) minutes. The
  filed time is read as its nearest occurrence within 12 hours (vTBFM manual §8.7). A filed time
  that has passed, a blank one, or a malformed or `0000` time counts as pushing now
  (`predict::proposed_wheels_up`).
- A prefile with no scheduler time gets no proposed wheels-up. Airport flow (with the demand, AADC
  and GDP draft built on it), the FCA board and the departures list do not list it, and its route
  popup has no per-fix table. #865 removes prefiles from the surfaces left.

The 10 minutes is the gap vTBFM draws between a filed time, which is a pushback, and the wheels-up
it implies (vTBFM manual §17.2). Every time read off a trajectory with a proposed wheels-up is
itself proposed; the FCA board, airport flow and the departures list mark it with a `P`
(`wheels_up_proposed` on `FcaFlight`, `FlowFlight` and `DepartureFlight`).

## Climb, cruise and descent

`VerticalProfile::build` settles what the flight is doing once (`Plan`), then flies it from where it is
to the field in steps of `STEP_SEC` (`backend/src/feed/trajectory.rs:454`) seconds, the last one cut short
at the field. Every surface reads that one track: a time, the groundspeed it was flown at and the
altitude there describe the same step.

```
once:      cruise altitude ─▶ cruise speed ─▶ descent start ─▶ level or climbing ─▶ scale or ceiling
each step: distance to go + altitude ─▶ climb | cruise | descent ─▶ true airspeed
              ─▶ wind at that point ─▶ groundspeed ─▶ next distance and altitude
```

**Cruise altitude** is the filed altitude, capped at a row's service ceiling, and never below where the
flight is. An ERAM record gives no ceiling: its maximum altitudes are not performance limits (a C172's
is 45,000 ft).

**Descent start** is the larger of two distances: the type's own still-air descent distance from cruise
to the field's elevation plus `DESCENT_MARGIN_NM` (`backend/src/feed/trajectory.rs:458`) nautical miles, and
where the descent path reaches cruise. The descent distance is summed in steps of `DISTANCE_STEP_FT`
(`backend/src/feed/performance.rs:24`) feet.

**The phase of each step**, decided again every step:

1. Descent, inside the descent start, unless the flight is still climbing under the rule below.
2. Cruise, within `CRUISE_BAND_FT` (`backend/src/feed/trajectory.rs:460`) feet of the level it flies, or above it.
3. Climb, otherwise.

**A short trip.** A flight on the ground, or an airborne one whose straight-line distance from its departure is less than
its distance to go, climbs until it meets the descent path, wherever that is. Any other flight inside the
descent start is predicted level or descending. An airborne flight from an unknown departure is one of
these.

**Level below the filed altitude.** One VATSIM position carries no vertical rate, so level is read from
where the flight is. An airborne flight outside its descent start, more than 500 ft below its
cruise altitude, and farther from its departure than `LEVEL_CLIMB_DISTANCES`
(`backend/src/feed/trajectory.rs:463`) times its type's still-air climb distance, stays at its present
altitude until the descent path reaches it. Nearer than that, or from an unknown departure, it climbs.

**Speeds and altitudes:**

| Phase | True airspeed | Altitude |
| --- | --- | --- |
| Climb | The ERAM climb table, or the row's indicated-airspeed schedule held down to its Mach number | Rises at the table's or row's rate, no higher than cruise or the descent path |
| Cruise | The cruise speed above, held down to a row's cruise Mach | Constant |
| Descent | The ERAM descent table, or the row's schedule, never faster than the cruise speed | The descent path, or the present altitude while that is lower |

The **descent path** rises `DESCENT_FT_PER_NM` (`backend/src/feed/trajectory.rs:456`) feet a nautical mile, 1,000 ft
in 3 nm, from the arrival field's elevation. A flight above it is on it at the next step; a flight below it
stays level until it gets there. A descent rate never moves the altitude: it only sets the type's own descent
distance.

ERAM's tables are standard-day. Their climb and descent speeds are multiplied by the square root of the
air temperature over the standard one where the winds carry a temperature
(`performance::temperature_factor`); the cruise speed is not. A row's Mach number reads the same
temperature (#836). No step flies slower than `GS_FLOOR_KT` (`backend/src/feed/trajectory.rs:452`) knots of
groundspeed. The default row climbs at 250 KIAS and 2,000 fpm below 10,000 ft, 290 KIAS and 1,500 fpm
above, and descends at 290 then 250 KIAS (`AircraftProfile::default`).

## Winds

The winds are NOAA's Rapid Refresh (RAP) model on a regular grid (#836), refreshed every
`WINDS_REFRESH_INTERVAL` (`backend/src/jobs.rs:147`) and replaced by still air once older than
`WINDS_MAX_AGE_SECS` (`backend/src/jobs.rs:154`) seconds. `Winds::along` samples the field along
the flight's path no more than `KNOT_SPACING_NM` (`backend/src/feed/winds.rs:20`) nautical miles
apart, and the profile reads each sample at its own altitude there, so a climb meets the low-level
wind and the cruise the jet stream. Groundspeed is the true airspeed less the headwind, held
between 0.4 and 1.6 times the true airspeed (`trajectory::effective_gs`). A route outside the
field is still air.

## Groundspeed anchoring

An airborne flight within 500 ft of the level it flies, or above it, is anchored to what it is flying:
every predicted groundspeed for the rest of the flight is scaled by its observed groundspeed over the
predicted one, held within `ANCHOR_MAX_BIAS` (`backend/src/feed/trajectory.rs:468`) of 1, so 0.85 to 1.15
(#313). The predicted one is its cruise speed at its altitude, less the headwind along its reported
heading. A flight read as level below its filed altitude is anchored at that level.

A flight that starts inside its descent start and is not climbing to meet the path may be slower than
predicted by more than `ASSIGNED_SPEED_DEADBAND_KT` (`backend/src/feed/trajectory.rs:465`) knots of calibrated
airspeed. It is then taken to be on an assigned speed: its observed calibrated airspeed is a ceiling to the
end of the route, and it is not scaled as well. A faster reading changes nothing.

Both are read fresh from each poll; nothing is carried between polls. A climbing or ground flight is
neither scaled nor held.

## Freeze horizons

OIS re-predicts a flight on every request, from the latest VATSIM publish, before and after any
freeze. What freezes is the schedule, in three places:

- an FCA release: its crossing time (`MeterInput::frozen_ms`) pins the flight in metering;
- an issued CFR: airport flow reserves its slot at the later of the CFR and the wheels-up the
  flight is held to, plus the airborne ETE;
- a published GDP: its slots are frozen at publish.

There is no freeze horizon, a distance or time inside which an unreleased flight's slot stops
moving. #817 [The metering scheduler never re-checks a fixed slot and has no freeze horizon, so
released and late flights sit inside the required gap] tracks adding one for FCA metering.

## The release time

A release, EDCT or CFR is a wheels-up time: the slot at the constraint less the airborne ETE, the
trajectory's ETA there less its wheels-up (`release_ms` in `handlers/flow.rs`, `write_meter` in
`feed/flow.rs`, `feed/gdp.rs`; #815).

- **RDY** means the pilot is ready at the runway now: the slot search starts at now plus the
  airborne ETE.
- **SET** and a CFR's `ready_time` are runway times: the release is never earlier than the ready
  time.
- **A lock held elsewhere** (an issued CFR, a published GDP slot, another FCA's release) floors
  RDY and SET for a ground flight: the search starts at the later of the ready time and that lock
  (`repos::flow::locked_wheels_up_elsewhere`). The flight's own release on the FCA does not, and an
  airborne flight, which has left, is not floored.
- **A CFR issued without a ready time** locks the program's metered CFR, else the flight's locked
  or proposed wheels-up (`FlowFlight.etd`).
- **FCA metering** keeps `RELEASE_MARGIN_MS` (`backend/src/handlers/flow.rs:515`) milliseconds on
  top of the separation, so a release never takes a slot that only just fits. Miles-in-trail turns
  into a time gap at the crossing speed, floored at `MIN_CROSS_SPEED_KT`
  (`backend/src/feed/predict.rs:286`) knots; airport-flow MIT reads that speed `MIT_CROSSING_NM`
  (`backend/src/feed/flow.rs:24`) nautical miles short of the field.

The FAA holds a CFR release to an airborne window from 2 minutes before to 1 minute after the
release time, and an EDCT to 5 minutes either side (JO 7110.65, 4-3-4). OIS stores the time; it
does not enforce the window.

## How TBFM and vTBFM differ

| | OIS | TBFM | vTBFM |
| --- | --- | --- | --- |
| Route | The filed route, else the great circle; padded 1.12 on the ground when unresolved | An adapted route, typically direct to the meter fix (Swenson et al. 1997, §2.1) | The filed route, from NASR and CIFP (manual §13.3, §15) |
| Performance | ERAM climb and descent tables for 2,692 types, else a family, wake-class or default row; a type's own row comes first | The trajectory synthesizer couples the route with the aircraft's energy state, atmospheric data and aircraft-specific performance models (Swenson §2.1) | ERAM climb and descent tables for 2,692 types, else ERAM's default record for the filed altitude (manual §13.4) |
| Winds | RAP grid, read at each point's altitude | 13 km RUC winds, hourly (ATC-364 §3.2.4) | RAP, headwind component only, groundspeed floored at 80 kt (manual §13.2, §16.5) |
| Integration | 15 s steps to the field | Re-predicted on each track or flight-plan update (Swenson §2.1) | 15 s steps, one jump per waypoint at cruise (manual §12.1, §16.5) |
| Anchoring | 0.85 to 1.15 within 500 ft of its level or above it, whole remaining flight; a slowed descent is held to its observed CAS | Not covered by the sources cited here | 0.85 to 1.15 within 500 ft of its level or above it, whole integration; a slowed descent is held to its observed CAS (`server/src/trajectory/integrator.ts:666-726`; manual §16.2 still names an older 250 kt gate) |
| Freeze | Releases, issued CFRs, published GDP slots; no horizon | The STA freezes at a site-adapted freeze horizon, 95 to 390 nm in the 2010 survey; the ETA keeps updating (ATC-364 §2, §3.2.1) | Per stream, in minutes: 20 by default on a fix stream, 0 on an EDC stream (manual §6.4.10) |
| Departure | Wheels-up at a ready time, else filed time + 10 min | The tower supplies the time the flight will be ready to depart the runway (NTRS 20180006743 §II) | ETE from the parking spot with no taxi (manual §18.3); unscheduled drawn at filed time + 10 min (§17.2) |

Where OIS's model departs from vTBFM's:

- A short trip climbs to meet the descent path. In vTBFM a flight whose whole route is inside top of
  descent is in descent from its first step.
- The descent path ends at the field's elevation; vTBFM's ends at sea level (manual §18.5).
- Descent is never faster than cruise (#361).
- ERAM's climb and descent speeds are scaled for temperature; vTBFM flies them standard-day.
- A type with neither a row nor a record flies the family, wake or default row.
- A filed speed is believed from 60 kt; vTBFM ignores 100 kt or less (`server/src/trajectory/performance.ts:133`).
- Cruise is stepped like the rest of the flight, and groundspeed is held to 0.4 to 1.6 times true
  airspeed with a 60 kt floor.
- STAR altitudes and speeds are not flown yet (#1100), and SID runway transitions are not spliced.

## Climbs measured on VATSIM

vTBFM measured that ERAM's nominal climbs cover 20 to 45 % more distance to FL350 than the A320
family, E75L, B752, B748 and most airliners fly on VATSIM; the B738 agrees within 3 % (manual
§18.2). A 45 min recording of the VATSIM feed on 2026-11-23 (177 publishes), over climbs seen from below
4,000 ft to at least 28,000 ft, gives the same picture. Medians; the observed distance is over the
ground with its wind and level-offs, ERAM's is still air:

| Type | Climbs | Observed | ERAM |
| --- | --- | --- | --- |
| A320 | 55 | 104 nm, 17.0 min | 148 nm, 22.1 min |
| A319 | 19 | 98 nm, 16.7 min | 131 nm, 19.4 min |
| B738 | 13 | 113 nm, 17.5 min | 104 nm, 16.2 min |
| B38M | 12 | 108 nm, 18.4 min | 106 nm, 16.5 min |
| B752 | 10 | 111 nm, 17.0 min | 129 nm, 19.9 min |

In the same recording jets climbed at 1.16 to 1.27 times ERAM's rate, by altitude band, and descended at 243 to
302 ft a nautical mile against the model's 333. ERAM's numbers are flown as they are. An operator's type row
replaces them for a type, and `aircraft-profile-builder` measures what each type flies on VATSIM and
scores a profile set against recorded flights (#828).

NASA's ATD-2 surface scheduler at Charlotte predicts the takeoff time from the airline's earliest
off-block time and still hands TBFM a runway time (NTRS 20170011508, §I, §III.B). OIS has no surface
model: a flight with no ready time takes the proposed wheels-up above.

## Sources

- Swenson, H. N., et al., "Design and Operational Evaluation of the Traffic Management Advisor at
  the Fort Worth Air Route Traffic Control Center," 1st USA/Europe ATM R&D Seminar, 1997, §2.1.
  <https://rosap.ntl.bts.gov/view/dot/12955/dot_12955_DS1.pdf>
- Robinson, M., Davison Reynolds, H., Evans, J., "Traffic Management Advisor (TMA) Weather
  Integration," MIT Lincoln Laboratory Project Report ATC-364, 2010, §2, §3.2.1, §3.2.4.
  <https://archive.ll.mit.edu/mission/aviation/publications/publication-files/atc-reports/Robinson_2010_ATC-364_WW-20740.pdf>
- Stevens, L., Callantine, T., Staudenmeier, M., "Evolution of Electronic Approval Request
  Procedures at Charlotte Douglas International Airport," DASC 2018 (NTRS 20180006743), §II, §III.
  <https://ntrs.nasa.gov/citations/20180006743>
- Stevens, L., et al., DASC 2017 (NTRS 20170011508), §I, §III.B.
  <https://ntrs.nasa.gov/api/citations/20170011508/downloads/20170011508.pdf>
- FAA JO 7110.65, paragraph 4-3-4, Departure Release, Hold for Release, Release Times.
- vTBFM manual, `docs/manual.md` in `zla-artcc/vTBFM` at `b6bc6c5d` (not public): §6.4.10, §8.7, §12.1,
  §13.2, §13.3, §13.4, §15, §16.2, §16.5, §17.2, §18.2, §18.3, §18.5; and the source files cited above, at the same
  commit.
