# Aircraft performance profiles

The [prediction](/reference/predictions) behind FCA metering, [arrival demand](/tmu/aadc), and runway
ETEs needs to know how each aircraft type climbs, cruises and descends. OIS has ERAM's performance tables built in
for 2,692 types, and flies those unless you say otherwise. This page is for the rest: a profile for a type
you want flown differently, or for the types ERAM doesn't have.

## Fields

- **Climb / descent speed schedule** — an `IAS-below-10k / IAS-above-10k` pair for each phase (e.g.
  `250/290` climbing, `290/250` descending), plus a climb rate range in feet/minute.
- **Cruise** — true airspeed or Mach, whichever the type is normally filed with.
- **Service ceiling** — caps the modeled cruise altitude, so a light aircraft is never timed as if
  it could reach the flight levels a jet would file. ERAM's tables carry no ceiling, but an
  altitude OIS can't read stops at the highest one they have a cruise speed for.

The descent rate sets how far out the descent speeds start. The descent itself is always 1,000 feet per 3 miles.

## Which one a flight flies

1. **A type profile** on this page. Adding one for a type replaces ERAM's tables for it, and deleting it
   puts ERAM's back.
2. **ERAM's tables**, if ERAM has the type.
3. **An engine-family profile** (jet, turboprop or piston). None exists until someone adds one with **Add
   family**, and few types reach it: only those the engine reference lists and ERAM doesn't.
4. **A wake-class profile.**
5. **The default.**

An FCA's debug view names the one each flight is on: `type:B738`, `eram:B738`, `family:jet`, `wake:H` or
`default`.

::: info Permissions
`flow.aircraft_profiles.read` to view, `flow.aircraft_profiles.update` to add or edit a profile.
:::
