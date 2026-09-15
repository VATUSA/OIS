# Aircraft performance profiles

The trajectory model behind FCA metering, [arrival demand](/tmu/aadc), and runway ETEs needs a
climb/cruise/descent performance profile per aircraft type — without one, every type is metered
like a generic jet, which under-times a light aircraft and over-times a heavy one. This page lets
staff define real profiles, editable by ICAO type.

## Fields

- **Climb / descent speed schedule** — an `IAS-below-10k / IAS-above-10k` pair for each phase (e.g.
  `250/290` climbing, `290/250` descending), plus a climb rate range in feet/minute.
- **Cruise** — true airspeed or Mach, whichever the type is normally filed with.
- **Service ceiling** — caps the modeled cruise altitude, so a light aircraft is never timed as if
  it could reach the flight levels a jet would file.

Resolution falls back in three tiers: an exact type match, then a profile for the type's wake
class, then a global default — a type with no profile of its own still gets a reasonable estimate
rather than failing outright.

::: info Permissions
`flow.aircraft_profiles.read` to view, `flow.aircraft_profiles.update` to add or edit a profile.
:::
