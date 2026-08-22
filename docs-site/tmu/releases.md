# Release times (CFR)

A **CFR** (Controlled Flight Rules release) is a wheels-up time you hand a departure so it fits the flow into a metered constraint. It's the ground-side companion to the metering itself.

## Two sources of a release

A CFR comes from either kind of constraint:

- **A metered destination ([GDP](/tmu/gdp))** — issued from the airport **Departures** board. OIS slots the flight into the first open arrival window at or after the pilot's ready time, clear of every other committed arrival.
- **A metering [FCA](/tmu/fcas)** — issued from the FCA map or the [IDST console](/tmu/idst). OIS slots the flight into the earliest open **crossing** slot for that FCA, spaced by its rate or miles-in-trail.

Either way the result is the same kind of frozen wheels-up time.

## Issuing a release

You have two ways to release a flight:

- **RDY (release earliest)** — OIS assigns the earliest open slot and freezes the wheels-up time that falls out of it. Use it when the pilot is ready now.
- **Set `HHMMz`** — you pin the wheels-up time to an exact minute and OIS holds the flight to it.

## Issued releases are frozen

Once issued, a release time is **locked**. It does not drift as traffic changes — only cancelling or re-issuing changes it. That's what lets a tower controller read the time off the strip and trust it.

::: tip Proposed vs. issued
Before you issue a release, a flight shows a **proposed** time that updates live — that's advisory. Only an **issued** release is frozen. The departures list and the IDST board both mark which is which.
:::

## Releases show up everywhere

An FCA-issued release also appears in the airport **Departures** view — in the **CFR** column and the "to metered fields" count — even when the destination has no GDP of its own. (When a flight is caught by both a GDP and an FCA, the GDP program's time wins.)

Releases sync **instantly across everyone**. Issuing or cancelling a release on one board nudges every other open board — the FCA map, IDST, and the departures view — so you never have to refresh to see another controller's release. (A short poll is the fallback if the live connection drops.)
