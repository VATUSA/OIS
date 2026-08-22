# Data & retention

Storing every aircraft's position every few seconds forever would be enormous, so OIS keeps **recent** traffic in full detail and progressively compresses the rest — while making sure anything you've **saved** stays pristine. This is why an old replay looks coarser than a fresh one.

## How long things are kept

| Age of the data | What's kept |
| --- | --- |
| **0–2 days** | Every position sample (~15 seconds apart) — full fidelity. |
| **2–14 days** | Thinned to roughly **one point per minute**. |
| **Older than 14 days** | Raw positions are removed; each flight keeps a **simplified track** (the route shape, turns and climbs preserved) plus its summary. |
| **Inside a saved capture** | **Everything, forever** — never thinned or removed. |

So for a detailed look back at an event, **save a capture** of it — that's what keeps full-resolution traffic permanently.

## What always survives

Even after the raw positions age out, each flight permanently keeps:

- its **simplified track** — a compact version of the path that preserves the shape (straight legs collapse, turns and climbs are kept),
- its **summary** — distance, duration, max altitude and groundspeed,
- its **plan history** — including any amendments.

So flight lookups and the flight track map keep working for old flights; only the second-by-second replay detail is what thins out over time.

::: warning Save what you want to keep in full
If an event is worth a detailed replay later, make sure it's recorded as a **saved capture** while it's fresh. Once a window has aged past the retention horizon, the fine-grained traffic for it is gone — the simplified tracks remain, but you can't recover the full-resolution replay.
:::
