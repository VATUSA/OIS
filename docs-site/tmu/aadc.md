# Arrival demand chart (AADC)

The **AADC** shows an airport's forward arrival push before it arrives, not just a single current
number — modeled on SIMTRAFFIC's AADC.

## Reading the chart

Arrivals over the next 4 hours are bucketed into **15, 30, or 60-minute** bars (pick the bucket
width from the toggle). Each bar is stacked by a **dimension** you choose:

- **Status** — airborne, on the ground, or proposed (not yet airborne).
- **Aircraft category** — wake/weight class.
- **Carrier** — the busiest operators in the window; everything else folds into **OTHER** so the
  chart stays readable at a busy international field.
- **Arrival fix** — which STAR/fix each arrival is filed via.

A dashed line marks the airport's current **AAR** (Airport Acceptance Rate) — the wind-favored rate
from that airport's [program](/tmu/restrictions#rate-programs) configuration — converted to a
per-bucket count so you can see at a glance which future buckets are already over capacity.

## Where to find it

A dedicated page under **Operations**, and as a widget you can add to a [custom board](/dashboard/boards)
— configured per-instance with its own airport, bucket size, and dimension.

::: info Permissions
Requires `tmu.program.read` — the same access that lets you view an airport's live program board.
:::
