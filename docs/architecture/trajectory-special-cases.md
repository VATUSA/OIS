# vTBFM special cases

The trajectory model is a port of vTBFM's (#985). vTBFM's integrator also carries rules for particular
cases, each with a reason. This page lists them so each can be taken, adapted or left out on its
own. File and line are in `zla-artcc/vTBFM` at `b6bc6c5d` (not public).

## Open

**Decision** is empty until an operator writes one. A case here is not built until its decision says so.

| # | vTBFM file:line | What it does | Why it is there | OIS today | Recommendation | Decision |
| --- | --- | --- | --- | --- | --- | --- |
| S10 | `server/src/trajectory/integrator.ts:72` | Groundspeed never below 80 kt | A headwind must not stall the integration | 60 kt, and 0.4 to 1.6 times true airspeed | Leave out: OIS's floor lets a light aircraft fly its own speed |  |
| S13 | `server/src/trajectory/integrator.ts:305-321` | No filed altitude flies the higher of its own altitude and FL350; a number under 1,000 is hundreds of feet | A ground flight needs a credible level and one already higher keeps its own | 35,000 ft, raised to where the flight is; a number of 600 or less is hundreds; a step climb reads its first level | Leave out |  |
| S16 | `server/src/trajectory/verticalPlan.ts:50` | The descent path ends at sea level | Displays read FL180 and above | It ends at the field elevation (#411) | Leave out |  |
| S17 | `server/src/trajectory/integrator.ts:1025` | One jump per cruise leg, on the wind sampled at its start | CPU | 15 s steps, wind at each | Leave out unless the steps measure slow |  |
| S18 | `server/src/trajectory/integrator.ts:592` | A step budget, and a flag on a track that used it up | A route with a bad fix can be very long | None: the route is bounded, and no step is slower than 60 kt | Leave out |  |
| S19 | `server/src/trajectory/sphere.ts:38` | Sphere math bit-identical to the turf library, and longitudes wrapped | Its golden files pin exact positions | `gc_dist` on the resolved path | Leave out |  |
| S20 | `server/src/upstream/rapWinds.ts:34` | 2.5 degree RAP mesh at nine altitudes, the edge cell held outside it, whole knots | One small download an hour | 1 degree RAP grid at 15 levels, still air outside it (#836) | Leave out |  |
| S21 | `server/src/shared/winds.ts:107` | Winds older than 12 hours are called stale and still flown | An old wind beats none | Still air once the field is too old | Leave out |  |
| S22 | `server/src/trajectory/integrator.ts:543` | A ground flight takes off now, from its stand, unless given a wheels-up | Its ready time is typed by a controller | The wheels-up it is locked to, else its filed time plus 10 min (#814) | Leave out |  |
| S23 | `server/src/routemods/apply.ts:52` | A ground flight within 10 nm of its origin keeps its whole route | Trimming by position dropped the first fixes of a route that starts behind the aircraft | The ground trim drops anchors by index, which can pass fixes close to the field | Adapt, after a test that shows a route losing a fix |  |
| S24 | `server/src/idst/idstScheduler.ts:370` | Landed means within 10 nm of the destination | A parked arrival must leave the departure list | Within 5 nm, on the ground | Leave out |  |
| S25 | `server/src/shared/routeParser.ts:424` | A fix more than 300 nm out that triples the leg is dropped | One name can belong to two fixes far apart | A leg over 900 nm is refused | Leave out |  |
| S26 | `server/src/shared/routeParser.ts:198` | A fix-radial-distance point is skipped | Its parser has no radials | Resolved (`NavData::resolve_frd`) | Leave out |  |
| S27 | `server/src/shared/routeParser.ts:53` | An out-of-date procedure number reads as the current one | Pilots file last cycle's procedure | The same | Nothing to take |  |
| S28 | `server/src/routemods/runwayTransition.ts:19` | The SID transition of the assigned departure runway is spliced in, and a stand-in trunk dropped | Some SIDs share no common route | None | Leave out until OIS holds an assigned departure runway |  |
| S29 | `server/src/routemods/transform.ts:32` | Facility rules rewrite a route with pseudo waypoints | A procedure that is always flown on a vector | None | Leave out |  |
| S30 | `server/src/shared/routeParser.ts:550` | A foreign destination comes from a world airport table | Its airport table is the FAA's | A foreign airport the table lacks ends the route at its last fix | Leave out |  |
| S31 | `server/src/shared/routeParser.ts:584` | A route is trimmed to the nearest leg, its first fix kept while ahead, heading not read | A heading is noisy near a fix | A fix behind the heading is skipped | Leave out |  |
| S32 | `server/src/shared/stars.ts:5` | STAR names per Southern California airport, and Las Vegas's | It serves those facilities' ladders | STARs from the navigation data | Leave out |  |
| S33 | `server/src/metering/meterArcs.ts:198` | Crossings of drawn metering arcs | Metering at a distance, not a fix | FCA lines | Leave out |  |
| S34 | `server/src/counts/sectorStrata.ts:25` | Sectors sampled along the track from FL180 up | Its sector monitor counts high sectors | `sector_tracks`, every altitude, minute by minute | Leave out |  |
| S35 | `server/src/trajectory/trajectoryCoordinator.ts:321` | Only flights a display needs are integrated | CPU on one server | Each request times the flights it lists | Leave out |  |
| S36 | `server/src/trajectory/trajectoryCoordinator.ts:388` | A flight only the monitor needs is integrated every third poll; results are memoized | CPU on one server | Per request, with a sector cache | Leave out |  |
| S37 | `server/src/trajectory/trajectoryCoordinator.ts:793` | Simulator sessions run on the feed's clock | Training replays | The publish time | Leave out |  |
| S38 | `server/src/routemods/apply.ts:139` | The fixes already passed are kept with the track | A ladder must say a fix was crossed | FCA metering drops a flight past the line | Leave out |  |
| S39 | `server/src/trajectory/performance.ts:356` | A class (jet, turboprop, piston) per type, an unknown type in every class | Ladder filters | `engine_family`, unknown types in none | Leave out |  |
| S40 | `server/src/trajectory/trajectoryCoordinator.ts:361` | The type is read from the FAA equipment string first | It is always filed | The short type first. The two read 1 of 1,605 live pilots apart on 2026-11-23 | Leave out |  |
| S41 | `server/src/trajectory/performance.ts:222` | Equal tables are shared in memory, and a count of defaulted types is exported | Memory, and a gauge of data gaps | One bundled table; the debug view names the source | Leave out |  |
| S42 | `server/src/routemods/apply.ts:62` | SID altitudes are not flown, and nothing corrects a climb | A climb-via needs a different mechanic | The same | Nothing to take |  |
| S44 | `server/src/trajectory/integrator.ts:94` | A climb stops within 500 ft of its level and holds it, so a flight filed at FL350 cruises up to 500 ft low | The band keeps the phase from flapping | The same | Adapt: step to the filed level, if an altitude reading low by 500 ft matters on a display |  |
| S45 | `server/src/trajectory/integrator.ts:117` | Every descent is 1,000 ft per 3 nm, 333 ft a nautical mile | The path pins the profile to the field | The same. Jets recorded on VATSIM on 2026-11-23 flew 243 to 302 ft a nautical mile | Leave out until the backtest scores a shallower path |  |

## Settled before the build

Taken, adapted or declined while #985 was planned, or already the same in OIS. #1100 carries the STAR restrictions.

| # | vTBFM file:line | What it does | Why it is there | Outcome |
| --- | --- | --- | --- | --- |
| S0 | `server/src/trajectory/performance.ts:248` | ERAM tables for 2,692 types | Replaced learned profiles for 15 types | Taken: bundled, behind an operator's type row |
| S1 | `server/src/trajectory/integrator.ts:705-726` | A slowed descent keeps its observed CAS, 15 kt deadband | Metering delay never counted down on a slowed flight | Taken |
| S2 | `server/src/trajectory/integrator.ts:344` | Below filed and past 1.5 times the climb distance is level | Stepped-down arrivals were predicted climbing back | Taken |
| S3 | `server/src/trajectory/verticalPlan.ts:50` | STAR altitude windows bend the descent path | Boundary altitudes | #1100 |
| S4 | `server/src/trajectory/integrator.ts:986` | STAR speeds are caps | ETAs on charted speeds | #1100 |
| S5 | `server/src/trajectory/integrator.ts:951` | Inside top of descent is descent, even if never climbed | One rule for every step | Adapted: a short trip climbs to meet the path |
| S6 | `server/src/trajectory/integrator.ts:108` | Descent speeds start 10 nm early | Early is safer than late | Taken; its fallback for a type with no descent table is not, since every record has one |
| S7 | `server/src/trajectory/integrator.ts:1096` | A flight above the path is on it at the next step | One line, no second descent model | Taken |
| S8 | `server/src/trajectory/integrator.ts:666` | The scale has no 250 kt gate, a 500 ft band, counts above cruise and reads the wind along the heading | Turboprops got none | Taken |
| S9 | `server/src/routemods/apply.ts:71` | Published speeds come only from the last procedure the origin does not publish | A SID was flown as a descent | #1100 |
| S11 | `server/src/trajectory/performance.ts:150` | A filed speed is held inside the type's range, its midpoint when unusable | ERAM's treatment of a filed speed | Taken, with OIS's 60 to 700 kt as usable in place of over 100 |
| S12 | `server/src/trajectory/performance.ts:275` | An unknown type flies ERAM's default for its altitude | Every flight gets a record | Not taken: OIS's family, wake and default rows |
| S14 | `server/src/trajectory/integrator.ts:949` | Filed below the present altitude holds level | A climb never descends | OIS already did this |
| S15 | `server/src/trajectory/performance.ts:76` | Standard-day speeds, no temperature | ERAM's tables are standard-day | Adapted: climb and descent speeds scaled by the field's temperature |
| S43 | `server/src/trajectory/integrator.ts:994` | A vectored flight is timed direct to its next fix | Nothing in the feed says it is on a vector | OIS already did this |
