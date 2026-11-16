# Airport surface data

Draw and label an airport's **gates/parking positions**, **ramp/apron areas**, and **taxiways** on
an interactive surface map. Gates also feed the departure-runway prediction: a facility's gate
rule names the runway a departure parked at that gate uses, and IDST shows it beside the release.

The page also shows the airport's active runway ends (read-only here — manage them from the
[Runway Balancer](/tmu/runway-balancer)) for context while placing gates.

::: info Permissions
Editing requires `flow.surface_data.update`, scoped to the airport's owning facility.
:::
