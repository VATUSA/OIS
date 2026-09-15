# Taxi & pushback insights

A staff view into the raw timing observations and derived estimates behind OIS's taxi/pushback
model — the same model that feeds every ETA and metering calculation that needs to know how long an
aircraft takes to get from the gate to the runway (and back, for arrivals).

## Observations

Every observed pushback and taxi duration, one row per flight: airport, gate, aircraft type,
runway, pushback and taxi time, and when it was recorded. Filter by any of those fields and a time
range; an **outlier** flag marks a row outside the model's sanity bounds (still shown by default —
toggle it off to hide them).

## Estimates

The model's current learned estimate per (airport, gate, aircraft, runway) combination — a
pushback and taxi duration, each tagged with the **tier** it resolved from, most specific first:

1. **Gate + type + runway** — enough samples for this exact combination.
2. **Airport + runway** — falls back to every aircraft at this gate's runway.
3. **Airport-wide** — falls back to the whole airport, ignoring gate and runway.
4. **Default** — no matching samples at all; a fixed fallback value.

A thin combo naturally resolves to a broader tier — that's expected, not a bug. Filter to a
specific fallback tier to audit which combos are still running on thin data.

::: info Permissions
Requires `stats.data.read`.
:::
