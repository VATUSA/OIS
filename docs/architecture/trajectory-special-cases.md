# vTBFM special cases

The trajectory model is a port of vTBFM's (#985). vTBFM's integrator also carries rules for particular
cases, each with a reason. This page lists them so each can be taken, adapted or left out on its
own: every rule in vTBFM's trajectory, route and wind code that changes a predicted path, altitude, speed or time, or
which flights are predicted. A rule both already have is listed as nothing to take. File and line are in `zla-artcc/vTBFM` at `b6bc6c5d` (not public).

## Open

**Decision** is empty until the operator answers. A case here is not built until it does; a decided row moves to
Decided.

| # | vTBFM file:line | What it does | Why it is there | OIS today | Recommendation | Decision |
| --- | --- | --- | --- | --- | --- | --- |
| S10 | `server/src/trajectory/integrator.ts:72` | Groundspeed never below 80 kt | A headwind must not stall the integration | 60 kt, and 0.4 to 1.6 times true airspeed | Leave out: OIS's floor lets a light aircraft fly its own speed |  |
| S13 | `server/src/trajectory/integrator.ts:305-321` | No filed altitude flies the higher of its own altitude and FL350; a number under 1,000 is hundreds of feet | A ground flight needs a credible level and one already higher keeps its own | 35,000 ft, raised to where the flight is; a number of 600 or less is hundreds; a step climb reads its first level; a filed 0 is unfiled | Leave out |  |
| S16 | `server/src/trajectory/verticalPlan.ts:75` | The descent path ends at sea level | Displays read FL180 and above | It ends at the field elevation (#411) | Leave out |  |
| S17 | `server/src/trajectory/integrator.ts:1000` | One jump per cruise leg, on the wind sampled at its start | CPU | 15 s steps, wind at each | Leave out unless the steps measure slow |  |
| S18 | `server/src/trajectory/integrator.ts:592` | A step budget, and a flag on a track that used it up | A route with a bad fix can be very long | A route over 25,000 nm is flown in longer steps, and no step is slower than 60 kt | Leave out |  |
| S19 | `server/src/trajectory/integrator.ts:379` | Sphere math bit-identical to the turf library, longitudes wrapped after each step | Its golden files pin exact positions | `gc_dist` on the resolved path | Leave out |  |
| S20 | `server/src/upstream/rapWinds.ts:34` | 2.5 degree RAP mesh at nine altitudes, the edge cell held outside it, whole knots | One small download an hour | 1 degree RAP grid at 15 levels, still air outside it (#836) | Leave out |  |
| S21 | `server/src/shared/winds.ts:107` | Winds older than 12 hours are called stale and still flown | An old wind beats none | Still air once the field is too old | Leave out |  |
| S22 | `server/src/trajectory/integrator.ts:543` | A ground flight takes off now, from its stand, unless given a wheels-up | Its ready time is typed by a controller | The wheels-up it is locked to, else its filed time plus 10 min (#814) | Leave out |  |
| S23 | `server/src/routemods/apply.ts:52` | A ground flight within 10 nm of its origin keeps its whole route | Trimming by position dropped the first fixes of a route that starts behind the aircraft | The ground trim drops anchors by index, which can pass fixes close to the field | Adapt, after a test that shows a route losing a fix |  |
| S24 | `server/src/idst/idstScheduler.ts:370` | Landed means within 10 nm of the destination | A parked arrival must leave the departure list | Within 5 nm, on the ground | Leave out |  |
| S25 | `server/src/shared/routeParser.ts:424` | A fix more than 300 nm out that triples the leg is dropped | One name can belong to two fixes far apart | A plain fix or navaid more than 900 nm from the last point is refused; a duplicated name takes the nearest | Leave out |  |
| S26 | `server/src/shared/routeParser.ts:198` | A fix-radial-distance point is skipped | Its parser has no radials | Resolved (`NavData::resolve_frd`) | Leave out |  |
| S27 | `server/src/shared/routeParser.ts:53` | An out-of-date procedure number reads as the current one | Pilots file last cycle's procedure | A bare or old name takes the shortest key with that prefix, which is not always the current revision | Adapt: take the current revision |  |
| S28 | `server/src/routemods/runwayTransition.ts:19` | The SID transition of the assigned departure runway is spliced in, and a stand-in trunk dropped | Some SIDs share no common route | None, except a runway transition filed as its own token before the SID. The departure runway OIS assigns enters no ETA (#814) | Leave out: it needs a decision that the assigned runway may move an ETA |  |
| S29 | `server/src/routemods/transform.ts:32` | Facility rules rewrite a route with pseudo waypoints | A procedure that is always flown on a vector | None | Leave out |  |
| S30 | `server/src/shared/routeParser.ts:550` | A foreign destination comes from a world airport table | Its airport table is the FAA's | The table is worldwide; a foreign route ends at the coverage edge, an unresolved token or a STAR, then goes direct | Leave out |  |
| S31 | `server/src/shared/routeParser.ts:584` | A route is trimmed to the nearest leg, its first fix kept while ahead, heading not read | A heading is noisy near a fix | A fix within 3 nm of the leg's end is passed, and one over 5 nm off and 110 degrees behind the heading is skipped | Leave out |  |
| S32 | `server/src/shared/stars.ts:5` | STAR names per Southern California airport, and Las Vegas's | It serves those facilities' ladders | STARs from the navigation data | Leave out |  |
| S33 | `server/src/metering/meterArcs.ts:198` | Crossings of drawn metering arcs | Metering at a distance, not a fix | FCA lines | Leave out |  |
| S34 | `server/src/counts/sectorStrata.ts:25` | Sectors sampled along the track from FL180 up | Its sector monitor counts high sectors | `sector_tracks`, every altitude, minute by minute | Leave out |  |
| S35 | `server/src/trajectory/trajectoryCoordinator.ts:321` | Only flights a display needs are integrated | CPU on one server | Each request times the flights it lists | Leave out |  |
| S36 | `server/src/trajectory/trajectoryCoordinator.ts:388` | A flight only the monitor needs is integrated every third poll; results are memoized | CPU on one server | Per request; sector demand and the traffic list are cached per publish | Leave out |  |
| S37 | `server/src/trajectory/trajectoryCoordinator.ts:793` | Simulator sessions run on the feed's clock | Training replays | Sector demand starts at the publish time, every other surface at the request | Leave out |  |
| S38 | `server/src/routemods/apply.ts:139` | The fixes already passed are kept with the track | A ladder must say a fix was crossed | FCA metering drops a flight past the line | Leave out |  |
| S39 | `server/src/trajectory/performance.ts:356` | A class (jet, turboprop, piston) per type, an unknown type in every class | Ladder filters | `engine_family`, unknown types in none; the jets-only rule keeps an unknown type unless it is wake L | Leave out |  |
| S40 | `server/src/trajectory/trajectoryCoordinator.ts:361` | The type is read from the FAA equipment string first | It is always filed | The short type first; with none, a heavy prefix is read as the type. The two read 1 of 1,605 live pilots apart on 2026-11-23 | Leave out |  |
| S41 | `server/src/trajectory/performance.ts:222` | Equal tables are shared in memory, and a count of defaulted types is exported | Memory, and a gauge of data gaps | One bundled table; the debug view names the source | Leave out |  |
| S42 | `server/src/routemods/apply.ts:65` | SID altitudes are not flown, and nothing corrects a climb | A climb-via needs a different mechanic | The same | Nothing to take |  |
| S44 | `server/src/trajectory/integrator.ts:94` | A climb stops within 500 ft of its level and holds it, so a flight filed at FL350 cruises up to 500 ft low | The band keeps the phase from flapping | A climb captured in the band goes on to its level | Leave out: a flight filed at a sector floor would be counted in the sector below |  |
| S14 | `server/src/trajectory/integrator.ts:949` | A flight filed below where it is holds its altitude | A climb never descends | It holds too, but its descent starts from where it is, where vTBFM keeps the filed top of descent and then snaps down | Leave out |  |
| S43 | `server/src/trajectory/integrator.ts:570` | A vectored flight is timed from where it is straight to its next fix | Nothing in the feed says it is on a vector | The same | Nothing to take |  |
| S46 | `server/src/trajectory/trajectoryCoordinator.ts:420` | Sector crossings stop being sampled past a horizon | No display looks further | Fixes stop at the sector monitor's six hours | Nothing to take |  |
| S47 | `server/src/trajectory/performance.ts:134` | A flight with no record flies its filed speed when over 100 kt, else 280 kt, and climbs at 1,500 fpm, unscaled | A path only its test fixtures take | Every flight resolves to a record or a row | Leave out |  |
| S48 | `server/src/shared/routeParser.ts:462` | With no destination, a last leg over 300 nm and three times the rest of the route is dropped | A name collision at the end of a route | The 900 nm refusal of S25 | Leave out |  |
| S49 | `server/src/shared/routeParser.ts:495` | A route with no resolvable point gives no track | No reason stated | The great circle to the field, padded on the ground; 300 nm with neither | Leave out |  |
| S50 | `server/src/trajectory/integrator.ts:731` | A flight starts at its reported altitude, on the ground too | Plain start | A ground flight starts at 0 ft | Take: a departure from a high field is predicted too low on its way up |  |
| S51 | `server/src/shared/routeParser.ts:290` | An airway is flown only between its listed entry and exit; unjoined, it is dropped | No reason stated | Joined at the waypoint nearest each end; with no fix between two airways the first is flown to its end | Take, with the parsing work |  |
| S52 | `server/src/shared/routeParser.ts:205` | A token joined by dots is split and each part read | `ORCKA5.MISEN` lost its procedure | None: the whole token is unresolved | Take, with the parsing work |  |
| S53 | `server/src/shared/routeParser.ts:500` | An unreadable token is skipped; runways and squawks are filtered first | One bad token must not lose the rest | Skipped on a domestic route; on a foreign one the first unresolved token ends it | Leave out |  |
| S54 | `server/src/trajectory/trajectoryCoordinator.ts:424` | A flight under 50 kt for a poll keeps its last airborne track | A dip in groundspeed dropped it from the ladders | None: a slow airborne pilot is a ground flight on that request | Adapt, if a paused pilot is seen to drop out of a list |  |
| S55 | `server/src/trajectory/trajectoryCoordinator.ts:455` | A ground departure is integrated fresh, never from its stored track | A stale track hid a departure | No track is stored | Nothing to take |  |
| S56 | `server/src/trajectory/trajectoryCoordinator.ts:562` | A departure over 90 s past its wheels-up is no demand | It holds no place | A past lock is timed from now | Leave out |  |
| S57 | `server/src/trajectory/trajectoryCoordinator.ts:547` | A stored track is sector load only while freshly confirmed | Phantom crossings | Projected per publish from live pilots | Nothing to take |  |
| S58 | `server/src/upstream/rapWinds.ts:187` | The newest RAP analysis is flown for the whole track | The best estimate of now | The forecast valid at the nearest hour, for the whole track | Leave out |  |
| S59 | `server/src/upstream/rapWinds.ts:145` | Wind levels are placed at their height in that cycle | A fixed pressure table is worst when the weather matters | Standard-atmosphere pressure altitude | Leave out: VATSIM altitudes near cruise are pressure altitudes |  |
| S60 | `server/src/pollers/winds.ts:193` | A failed fetch keeps the field, and a restart adopts the cached one under 12 h old | A restart into a dead upstream | A field is kept 3 h; a restart boots on still air | Adapt: load the last stored field at boot |  |
| S61 | `server/src/upstream/rapWinds.ts:122` | A level missing a field is dropped, ends are held, a bad block is calm | Extrapolating past the top invents a 300 kt headwind | A file missing a wind level is refused for the next; ends held, gaps still | Leave out |  |
| S62 | `server/src/trajectory/trajectoryCoordinator.ts:805` | No winds means still air, never no track | Bailing out emptied the lists | The same | Nothing to take |  |
| S63 | `server/src/shared/winds.ts:204` | Wind is blended as vectors and held beyond 3,000 and 39,000 ft | Degrees wrap at 360 | The same idea | Nothing to take |  |
| S65 | `server/src/shared/constants.ts:14` | Airborne is 50 kt or more at any altitude | One value everywhere | The same | Nothing to take |  |
| S66 | `server/src/shared/routeParser.ts:534` | The destination is exempt from the detour test and judges the last fix | It was safe only by accident | Appended unguarded; it does not judge the last fix | Leave out |  |
| S67 | `server/src/shared/routeParser.ts:537` | A K is stripped only from a K airport | Stripping any letter sent YMML to Minnesota | An exact ICAO key | Leave out |  |
| S68 | `server/src/trajectory/integrator.ts:915` | A fix within 0.01 nm is recorded at once; steps are cut short and snapped at each fix | Rounding must not cost 15 s | One distance axis, cut short only at the field | Nothing to take |  |
| S69 | `server/src/counts/sectorStrata.ts:26` | Sector band edges at 23,500 and 34,500 ft, missing layers worked by another | A level flight flickered between layers | Each volume has its own base and top | Leave out |  |
| S71 | `server/src/trajectory/performance.ts:154` | A record with a table missing is flown on the ones it has; a bad file stops startup | 280 kt for every flight would be silent | A file missing any table is refused when first read, at startup | Nothing to take |  |
| S72 | `server/src/trajectory/integrator.ts:326` | Leading digits of the filed speed are read | The field is free text | A number or nothing, which is the midpoint | Take: `450kt` is readable |  |
| S73 | `server/src/shared/routeParser.ts:559` | A point at the previous position is dropped | No reason stated | The same | Nothing to take |  |
| S74 | `server/src/shared/routeParser.ts:431` | The first leg of the detour test is measured from the aircraft | The aircraft is the only honest anchor | From the departure field | Leave out |  |
| S75 | `server/src/trajectory/integrator.ts:1149` | A time outside the track reads its end; a number that is not one fails the flight | Keeps results finite | The same ends; inputs are bounded before the first step | Nothing to take |  |

## Decided

Answered by the operator during #985. **Decision** quotes the answer. The STAR cases are
#1100 [STAR crossing restrictions are dropped at the CIFP parser] (Triaging), the SID case
#1099 [SID exit transitions and SIDs without a common route expand to nothing] (Triaging).

| # | vTBFM file:line | What it does | Why it is there | OIS today | Recommendation | Decision |
| --- | --- | --- | --- | --- | --- | --- |
| S0 | `server/src/trajectory/performance.ts:248` | ERAM tables for 2,692 types | Replaced learned profiles for 15 types | The same file, behind an operator's type row | Take | "eram data is okay to put in the repo." |
| S1 | `server/src/trajectory/integrator.ts:705-726` | A slowed descent keeps its observed CAS, 15 kt deadband | Metering delay never counted down on a slowed flight | The same, except that a climbing flight and one whose departure is unknown are not held, and the scale is dropped | Take | "All four now" |
| S2 | `server/src/trajectory/integrator.ts:344` | Below filed and past 1.5 times the climb distance is level; the route midpoint with no climb table, and never on a round trip | Stepped-down arrivals were predicted climbing back | The same rule; every record has a climb table, so the midpoint is not needed | Take | "All four now" |
| S3 | `server/src/trajectory/verticalPlan.ts:50` | STAR altitude windows bend the descent path | Boundary altitudes | None | Take, in its own change | "All four now", then "Three issues": #1100 |
| S4 | `server/src/trajectory/integrator.ts:986` | STAR speeds are caps | ETAs on charted speeds | None | Take, with S3 | "All four now", then "Three issues": #1100 |
| S5 | `server/src/trajectory/integrator.ts:951` | Inside top of descent is descent, even if never climbed | One rule for every step | A short trip climbs to meet the path | Adapt | "Adapt" |
| S6 | `server/src/trajectory/integrator.ts:601` | Descent speeds start 10 nm early, or where the path reaches cruise if farther; 1 nm per 300 ft with no descent table | Early is safer than late | The same; every record has a descent table | Take the margin | "vTBFM's 10 nm" |
| S7 | `server/src/trajectory/integrator.ts:1101` | A flight above the path is on it at the next step | One line, no second descent model | The same | Take | "vTBFM's snap" |
| S8 | `server/src/trajectory/integrator.ts:666` | The scale has no 250 kt gate, a 500 ft band, counts above cruise and reads the wind along the heading; 15 percent either way, for the whole route | Turboprops got none | The same gates; the scale is on groundspeed, where vTBFM scales true airspeed before the wind | Take the gates | "vTBFM's gates" |
| S76 | `server/src/shared/routeParser.ts:266` | A SID's exit transition is chosen by the next filed fix and joined to its trunk; with no match it flies its trunk, else its longest path | `MUSEL8 SLI` flew 220 nm where 31 was filed | The transition is read only from the token before; a SID with no common route expands to nothing | Take | "Three issues": #1099 |
| S77 | `server/src/trajectory/procedureConstraints.ts:4` | The finer rules of S3, S4 and S9: read before the trim, last occurrence of a fix, a passed cap stays, a floor beats a contradicting ceiling, no runway transitions | A 280 kt cap was forgotten once its fix was passed | None | Take, with S3 | "Three issues": #1100 |
| S9 | `server/src/routemods/apply.ts:71` | Published speeds come only from the last procedure the origin does not publish | A SID was flown as a descent | No published speeds | Take, with S3 | "Three issues": #1100 |
| S11 | `server/src/trajectory/performance.ts:150` | A filed speed over 100 kt is held inside the range for its type, the midpoint when unusable | ERAM's treatment of a filed speed | The same for an ERAM type, with 60 to 700 kt usable and the range read once, at the level flown | Take, with OIS's bounds | "ERAM clamp + OIS bounds" |
| S12 | `server/src/trajectory/performance.ts:275` | An unknown type flies ERAM's default for its altitude | Every flight gets a record | Family, wake and default rows | Leave out | "OIS's tiers" |
| S15 | `server/src/trajectory/performance.ts:76` | Standard-day speeds, no temperature | ERAM's tables are standard-day | Climb and descent speeds scaled by the field's temperature | Adapt | "Scale ERAM speed" |
| S45 | `server/src/trajectory/integrator.ts:117` | Every descent is 1,000 ft per 3 nm, 333 ft a nautical mile | The path pins the profile to the field | The same. Jets recorded on VATSIM on 2026-11-23 flew 243 to 302 ft a nautical mile | Take; look again when the backtest scores a shallower path | "Approve as written" (AC 3) |
| S64 | `server/src/trajectory/integrator.ts:1102` | Below the path a descending flight stays level until it meets it | Keeps the descent monotonic | The same | Take | "Approve as written" (AC 2 and 3) |
| S70 | `server/src/trajectory/performance.ts:28` | The type's maximum altitude caps nothing | The tables run to 44,000 ft and more | The same; a row has a ceiling | Take | "No ceiling, keep descent cap" |
| S78 | `server/src/trajectory/integrator.ts:710` | A slowed descent is held whatever its departure | One rule for every flight inside top of descent | A flight whose departure is unknown is not held: it may be climbing out of it. A slowed arrival with a bad filing loses the hold, and an unheld one is what vTBFM measured reading about 5 min early | Adapt | "Fix, reword AC 9" |
