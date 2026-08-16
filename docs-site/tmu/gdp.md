# Ground Delay Programs

A **Ground Delay Program** (GDP) meters arrivals into an airport that can't accept its current demand, by holding departures on the ground and issuing each a controlled arrival time. OIS rations slots by schedule and **freezes** the resulting release times so tower controllers can rely on them.

## The idea

When demand exceeds an airport's **Airport Acceptance Rate** (AAR), OIS spaces arrivals one slot apart (`3600 ÷ AAR` seconds) and assigns each inbound the next open slot. Working back from that arrival slot gives each flight an **EDCT** (Expect Departure Clearance Time) — its wheels-up time. Delay is absorbed on the ground, not in a hold.

## Creating a program

1. Open the airport's **TMU** tab and start a GDP.
2. Set the **AAR** (arrivals per hour) and the program **window** (start/end, in Zulu).
3. Optionally add **AAR steps** — rate changes during the window (e.g. `30 → 60 @ 1500z`) for a graduated recovery.
4. Review the board — it previews each flight's slot, EDCT, and delay.

While a program is a **draft**, its control times are advisory and recompute live as traffic changes.

## Publishing & freezing

**Publish** the program to make it live. On publish, the EDCTs **freeze**: each controlled flight's control time is persisted and no longer moves on its own. This is what lets a tower controller read a release time off the strip and trust it.

## Managing a live program

- **Lock / unlock** a slot to pin or release an individual flight.
- **Compress** the program to pull frozen times **earlier** into slots vacated by cancellations or pop-ups — releases only ever move earlier under compression, never later.
- **Revise** the AAR, window, or steps; **extend** the window; or let it **auto-expire** at the end.

::: tip Frozen means frozen
A published EDCT never drifts on its own. It changes only when *you* revise, compress, or cancel the program — so controllers aren't surprised by a moving release time.
:::

## Scope

A GDP can be scoped to specific departure ARTCCs, so only flights from those centers are controlled.
