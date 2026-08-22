# Replay

Replay puts a past window of traffic back on the map so you can **scrub through time** — watch the arrivals build, see where the flow initiatives bit, and review exactly how an event played out.

## Captures vs. custom windows

There are two ways to pick what to replay:

- **A saved capture** — a window someone recorded (often tied to an event). Captures are kept at **full fidelity forever**, so they're the best source for a detailed review. Open one from the **Saved captures** list on the [Historical](/historical/overview) page.
- **A custom window** — any `from` / `to` time range. This replays whatever traffic is still retained for that period (see [Data & retention](/historical/retention)), so recent windows are sharp and older ones are coarser.

## Scrubbing

Drag the scrubber (or play) to move the clock. As it moves:

- **Traffic** animates — each aircraft interpolated between its recorded samples.
- **Clicking a flight** shows its track table (time, altitude, groundspeed, position) at the current instant.
- **Labels** can be toggled to show departure/arrival on each aircraft.

## Plans reflect the moment

If a flight **amended its plan** mid-window — say it diverted from KBOS to KPHL — the replay shows the plan that was **actually in effect at the clock**: KBOS before the amendment, KPHL after. The selected-flight panel lists the **plan amendments** with the time each took effect, so a diversion reads correctly instead of being retroactively rewritten.

Winds are replayed from the snapshot that applied at each instant too, so historical ETAs use the winds that were really there — not today's.

::: tip Flow initiatives
The time-machine dashboard pairs a replay with a board, so you can watch the TMIs, GDPs, and ground stops that were active alongside the traffic.
:::
