# Ideas / backlog

Unscheduled ideas not yet in [PLAN.md](PLAN.md). Promote to a feature spec + plan when picked up.

## Configurable aircraft performance profiles — _implemented on `feat/aircraft-profiles`_

Today the ETA/trajectory model ([`backend/src/feed/trajectory.rs`](../backend/src/feed/trajectory.rs))
uses one hard-coded climb/cruise profile for every aircraft, so a C172 is metered as if it
had B77W performance. Let staff define per-ICAO-type performance profiles, editable in the
UI, applied when a matching type is found and falling back to a default otherwise.

**Granularity — model it the way SimBrief does** (see the SimBrief "Airframe Performance"
panel):

- **Climb profile** — a speed schedule `IAS_below_10k / IAS_above_10k / Mach` (e.g. `250/280/.78`),
  plus climb rates.
- **Descent profile** — `Mach / IAS_above_10k / IAS_below_10k` (e.g. `.78/280/250`), plus a
  descent rate. Needed so aircraft that cross an FCA while descending toward arrival are timed
  with descent speeds, not cruise.
- **Cruise** — cruise TAS or Mach.
- **Service ceiling** — cap the modeled cruise altitude (a C172 never sees FL350).

Non-goal: takeoff / runway performance — irrelevant to flow timing.

Applies to every surface that shares the trajectory model: FCA metering, airport-flow demand,
runway ETE.
