# Statistics & replay

The **Historical** area is OIS's time machine. It keeps a continuous record of US VATSIM traffic so you can look back at what happened — network trends, an airport's activity, a single flight's track — and **replay** any past window on the map.

It's a controller/staff tool: viewing it needs the **`stats.data.read`** permission.

## What's here

- **Network statistics** — pilots online over the last **24h / 7d / 30d**, and the busiest airports.
- **Airport activity** — search a field (e.g. `KATL`) for its top aircraft, top origins and destinations, and recent movements.
- **Flight history** — look up a flight to see its track, summary (distance, duration, max altitude/speed), and any **plan amendments** it filed.
- **Saved captures** — recorded event/time windows you can [replay](/historical/replay) on the map.
- **[Taxi & pushback insights](/historical/taxi-insights)** — browsable observed and learned taxi/pushback timing, by gate, aircraft, and runway.
- **[Delays](/historical/delays)** — arrival and departure delay history, filterable by airport, runway, or procedure.

## How it's collected

OIS samples the live VATSIM feed continuously and stores each flight's position, the network totals, and the winds aloft. It records **US traffic** by default; while a [capture](/historical/replay#captures-vs-custom-windows) is open it records the whole network.

::: tip Fidelity isn't forever
Recent traffic is kept at full detail; older data is progressively thinned and eventually reduced to a simplified track — except anything inside a **saved capture**, which is kept in full forever. See [Data & retention](/historical/retention).
:::
