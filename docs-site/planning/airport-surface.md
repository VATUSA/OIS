# Airport surface data

Draw and label an airport's **gates/parking positions**, **ramp/apron areas**, and **taxiways** on
an interactive surface map. This isn't just reference data — it's what the taxi/pushback timing
model learns from: every observed pushback and taxi time is tied to a gate, so
[Taxi & pushback insights](/historical/taxi-insights) can build a per-gate estimate instead of one
flat number for the whole airport.

The page also shows the airport's active runway ends (read-only here — manage them from the
[Runway Balancer](/tmu/runway-balancer)) for context while placing gates.

::: info Permissions
Editing requires `flow.surface_data.update`, scoped to the airport's owning facility.
:::
