# Data & retention

Storing every aircraft's position every few seconds forever would be enormous, so OIS keeps **recent** traffic in full detail and progressively compresses the rest — while making sure anything you've **saved** stays pristine. This is why an old replay looks coarser than a fresh one.

## How long things are kept

| Age of the data | What's kept |
| --- | --- |
| **0–7 days** | Every position sample (~15 seconds apart) — full fidelity. |
| **7–14 days** | Thinned to roughly **one point per minute**. |
| **14–21 days** | Thinned again, to roughly **one point every 4 minutes**. |
| **Older than 21 days** | Roughly **one point every 16 minutes**, kept at that density from then on. |
| **Inside a saved capture** | **Everything, forever** — never thinned. |

**Positions are never deleted, only thinned.** Even a years-old flight keeps a coarse position trail,
alongside the simplified track and summary described below.

So for a detailed look back at an event, **save a capture** of it — that's what keeps full-resolution
traffic permanently. The flip side is that a saved capture pins that data indefinitely, so a capture
saved by mistake is storage nobody gets back until it is deleted (see below).

## What always survives

Even after the raw positions age out, each flight permanently keeps:

- its **simplified track** — a compact version of the path that preserves the shape (straight legs collapse, turns and climbs are kept),
- its **summary** — distance, duration, max altitude and groundspeed,
- its **plan history** — including any amendments.

So flight lookups and the flight track map keep working for old flights; only the second-by-second replay detail is what thins out over time.

::: warning Save what you want to keep in full
If an event is worth a detailed replay later, make sure it's recorded as a **saved capture** while it's fresh. Once a window has aged past the retention horizon, the fine-grained traffic for it is gone — the simplified tracks remain, but you can't recover the full-resolution replay.
:::

## Deleting a saved capture

Because a saved capture is what keeps its window at full fidelity, deleting one is also how that
storage is released. Anyone holding `stats.capture.delete` can remove a capture from the replay
page; its positions stop being protected and rejoin the thinning ladder above on the next
compaction pass, so the space comes back gradually rather than at once.

A capture belonging to an event can be deleted too, but the confirmation names the event first —
that event's replay and debrief read from the same window.
