# The prediction model

OIS predicts when a flight reaches a point: an FCA line, a fix, its arrival field. One model does
it, `feed/trajectory.rs`, behind one entry point, `feed::predict::trajectory`, which every surface
calls (#813). The callers and the command that lists them are in
[`docs/features/flow.md` → The ETA model and its callers](../features/flow.md#the-eta-model-and-its-callers).
This page says what the model is: what it reads, how it flies a flight, what a release time means,
and where it differs from FAA TBFM and from vTBFM.

Constants are cited as `NAME` (`path:line`), the line that defines them, paths relative to the
repository root. `docs_tests::the_prediction_doc_cites_each_constant_where_it_is_defined` fails
when a cited line no longer defines its constant.

## What the model reads

For each flight:

- **Its position**, for a connected pilot: latitude, longitude, heading, groundspeed and altitude
  from the VATSIM feed, polled every `FAST_POLL_SECS` (`backend/src/feed/mod.rs:64`) seconds and
  every `STALE_POLL_BACKOFF_SECS` (`backend/src/feed/mod.rs:72`) while the feed repeats itself. A
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
- the aircraft performance profiles, resolved exact type, then wake class, then a default
  (`ProfileTable::resolve`).

The entry point decides each input once:

- **Airborne** means a groundspeed of at least `AIRBORNE_GS_KT` (`backend/src/feed/predict.rs:277`)
  knots, at any altitude.
- **The route** is the filed route resolved from where the flight is. When it does not resolve,
  the model flies the great circle to the arrival field. A flight not yet airborne on a route that
  resolves to its two endpoints only is padded by `GROUND_ROUTE_FACTOR`
  (`backend/src/feed/predict.rs:41`), since no airway routing flies the direct line. A ground or
  prefiled flight with no path to fly, a prefile from an unknown field or a route that does not
  resolve to an unknown arrival field, is timed over `NOMINAL_ROUTE_NM`
  (`backend/src/feed/predict.rs:289`) nautical miles. A connected ground pilot within `ARRIVED_NM`
  (`backend/src/feed/predict.rs:480`) of an arrival field it did not depart from has arrived: it
  gets no great circle to that field, and airport flow labels it arrived.
- **The filed altitude** is the first run of digits in the filing, so `FL350/370` is 35,000 ft; an
  unreadable filing is timed at 35,000 ft (`trajectory::parse_alt_ft`).
- **The filed cruise speed** is capped by the type's profile. A value above `MAX_PLAUSIBLE_TAS`
  (`backend/src/feed/trajectory.rs:17`) knots, or below 60, is replaced by the profile's cruise or
  an altitude default (`trajectory::capped_cruise_tas`).

## Wheels-up

Every trajectory starts at wheels-up and flies from the runway. No taxi time is added anywhere
(#814): taxi belongs to the tower, which hands the model a runway time.

- An airborne flight is wheels-up now.
- A ground flight held to a wheels-up is wheels-up then, or now if that time has passed. The hold is
  the latest of its issued CFR, its releases in live FCAs (RDY and SET each store one) and its
  published GDP slot (`repos::flow::locked_wheels_up`), which the FCA board, airport flow, the
  departures list and the per-fix table read for every flight (`TimingTables::live`). The GDP draft
  reads every lock but its own airport's GDP slots, so a revise or compress can still move a flight
  earlier (`repos::flow::locked_wheels_up_outside_gdps_at`). Such a flight's ETA runs from its
  lock, so its delay column shows only delay added after the lock; the delay it took on the ground
  is in its release time.
- A connected ground pilot with no scheduler time is wheels-up at its proposed time: its filed
  departure time plus `PROPOSED_WHEELS_UP_MIN` (`backend/src/feed/predict.rs:35`) minutes. The
  filed time is read as its nearest occurrence within 12 hours (vTBFM manual §8.6). A filed time
  that has passed, a blank one, or a malformed or `0000` time counts as pushing now
  (`predict::proposed_wheels_up`).
- A prefile with no scheduler time gets no proposed wheels-up. Airport flow (with the demand, AADC
  and GDP draft built on it), the FCA board and the departures list do not list it, and its route
  popup has no per-fix table. #865 removes prefiles from the surfaces left.

The 10 minutes is the gap vTBFM draws between a filed time, which is a pushback, and the wheels-up
it implies (vTBFM manual §18.2). Every time read off a trajectory with a proposed wheels-up is
itself proposed; the FCA board, airport flow and the departures list mark it with a `P`
(`wheels_up_proposed` on `FcaFlight`, `FlowFlight` and `DepartureFlight`).

## Climb, cruise and descent

`VerticalProfile::build` integrates a profile over the distance to the field:

- **Climb** from the surface (or from where an airborne flight is) on the profile's indicated
  airspeed schedule, one speed and rate below 10,000 ft and another above, converted to true
  airspeed through the standard atmosphere. With a Mach number set, the climb holds the lower of
  the IAS and the Mach true airspeed.
- **Cruise** at the filed altitude, capped at the profile's service ceiling, at the capped filed
  true airspeed, or the lower of that and the profile's cruise Mach in the air temperature there.
- **Descent** to the arrival field's elevation at the profile's descent rate and speed schedule,
  from a top of descent the profile computes. A route too short to reach cruise flies a climb and
  a descent that meet at the altitude that fills it.

Altitude is stepped every `VERT_STEP_FT` (`backend/src/feed/trajectory.rs:407`) feet to build the
climb and descent curves, and time is integrated every `HORIZ_STEP_NM`
(`backend/src/feed/trajectory.rs:409`) nautical miles along them. No step flies slower than
`GS_FLOOR_KT` (`backend/src/feed/trajectory.rs:405`) knots of groundspeed. The default profile
(no type or wake match) climbs at 250 KIAS and 2,000 fpm below 10,000 ft, 290 KIAS and 1,500 fpm
above, and descends at 290 then 250 KIAS and 1,800 fpm (`AircraftProfile::default`).

## Winds

The winds are NOAA's Rapid Refresh (RAP) model on a regular grid (#836), refreshed every
`WINDS_REFRESH_INTERVAL` (`backend/src/jobs.rs:143`) and replaced by still air once older than
`WINDS_MAX_AGE_SECS` (`backend/src/jobs.rs:150`) seconds. `Winds::along` samples the field along
the flight's path no more than `KNOT_SPACING_NM` (`backend/src/feed/winds.rs:20`) nautical miles
apart, and the profile reads each sample at its own altitude there, so a climb meets the low-level
wind and the cruise the jet stream. Groundspeed is the true airspeed less the headwind, held
between 0.4 and 1.6 times the true airspeed (`trajectory::effective_gs`). A route outside the
field is still air.

## Groundspeed anchoring

An airborne flight established at cruise, at least `ANCHOR_MIN_GS_KT`
(`backend/src/feed/trajectory.rs:412`) knots and within `ANCHOR_ALT_TOLERANCE_FT`
(`backend/src/feed/trajectory.rs:414`) feet of its cruise altitude, is anchored to what it is
flying: every predicted groundspeed for the rest of the flight is scaled by its observed
groundspeed over the predicted one, a ratio clamped to ±`ANCHOR_MAX_BIAS`
(`backend/src/feed/trajectory.rs:417`), so 0.85 to 1.15 (#313). The ratio is taken fresh from each
poll; nothing is carried between polls. A climbing or ground flight is not anchored.

## Freeze horizons

OIS re-predicts a flight on every request, from the latest VATSIM publish, before and after any
freeze. What freezes is the schedule, in three places:

- an FCA release: its crossing time (`MeterInput::frozen_ms`) pins the flight in metering;
- an issued CFR: airport flow reserves its slot at the locked wheels-up plus the airborne ETE;
- a published GDP: its slots are frozen at publish.

There is no freeze horizon, a distance or time inside which an unreleased flight's slot stops
moving. #817 [The metering scheduler never re-checks a fixed slot and has no freeze horizon, so
released and late flights sit inside the required gap] adds one for FCA metering.

## The release time

A release, EDCT or CFR is a wheels-up time: the slot at the constraint less the airborne ETE, the
trajectory's ETA there less its wheels-up (`release_ms` in `handlers/flow.rs`, `write_meter` in
`feed/flow.rs`, `feed/gdp.rs`; #815).

- **RDY** means the pilot is ready at the runway now: the slot search starts at now plus the
  airborne ETE.
- **SET** and a CFR's `ready_time` are runway times: the release is never earlier than the ready
  time.
- **A lock held elsewhere** (an issued CFR, a published GDP slot, another FCA's release) floors
  RDY and SET: the search starts at the later of the ready time and that lock
  (`repos::flow::locked_wheels_up_elsewhere`). The flight's own release on the FCA does not.
- **A CFR issued without a ready time** locks the program's metered CFR, else the flight's locked
  or proposed wheels-up (`FlowFlight.etd`).
- **FCA metering** keeps `RELEASE_MARGIN_MS` (`backend/src/handlers/flow.rs:482`) milliseconds on
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
| Route | The filed route, else the great circle; padded 1.12 on the ground when unresolved | An adapted route, typically direct to the meter fix (Swenson et al. 1997, §2.1) | The filed route, from NASR and CIFP (manual §14.3, §16) |
| Performance | Per-type, wake-class or default profile; climb, cruise, descent to a real top of descent | The trajectory synthesizer couples the route with the aircraft's energy state, atmospheric data and aircraft-specific performance models (Swenson §2.1) | ERAM climb and descent tables for 2,692 types (manual §14.4) |
| Winds | RAP grid, read at each point's altitude | 13 km RUC winds, hourly (ATC-364 §3.2.4) | RAP, headwind component only, groundspeed floored at 80 kt (manual §14.2, §17.5) |
| Integration | 500 ft vertical, 3 nm horizontal steps | Re-predicted on each track or flight-plan update (Swenson §2.1) | 15 s steps, one point per waypoint at cruise (manual §13.1, §17.5) |
| Anchoring | ±15 % at cruise (≥250 kt, within 300 ft), whole remaining flight | Not covered by the sources cited here | 0.85 to 1.15 at cruise (≥250 kt, within 500 ft), whole integration (manual §17.2) |
| Freeze | Releases, issued CFRs, published GDP slots; no horizon | The STA freezes at a site-adapted freeze horizon, 95 to 390 nm in the 2010 survey; the ETA keeps updating (ATC-364 §2, §3.2.1) | Per stream, in minutes: 20 by default on a fix stream, 0 on an EDC stream (manual §6.3.10) |
| Departure | Wheels-up at a ready time, else filed time + 10 min | The tower supplies the time the flight will be ready to depart the runway (NTRS 20180006743 §II) | ETE from the parking spot with no taxi (manual §19.3); unscheduled drawn at filed time + 10 min (§18.2) |

vTBFM measured that ERAM's nominal climbs cover 20 to 45 % more distance to FL350 than the A320
family, E75L, B752, B748 and most airliners fly on VATSIM; the B738 agrees within 3 % (manual
§19.2). OIS's profiles are rows an operator writes; `aircraft-profile-builder`
measures what each type flies on VATSIM and scores a profile set against recorded flights (#828).

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
- vTBFM manual, `docs/manual.md` in `zla-artcc/vTBFM` at `906db6ca` (not public): §6.3.10, §8.6, §13.1,
  §14.2, §14.3, §14.4, §16, §17.2, §17.5, §18.2, §19.2, §19.3.
