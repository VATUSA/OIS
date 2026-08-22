# Flow Constrained Areas

A **Flow Constrained Area** (FCA) is a **line you draw across a flow** to identify and meter the traffic crossing it — the building block for most flow initiatives in OIS. Aircraft are metered as their filed route **crosses the line**, so an FCA is an open boundary, not a closed area.

## Building an FCA

1. Open the FCA map and start a new area.
2. **Draw the line** on the map — click to add points along it; the redraw button lets you start the line over.
3. Name it and set its **filters** — which traffic counts. Every filter is optional; leaving one blank means "don't filter on it":
   - **Destination airports** — only meter arrivals to these fields (blank = every arrival crossing the line).
   - **Departure airports** — only meter traffic *out of* these fields.
   - **Route fixes** — only meter aircraft whose filed route contains these fixes.
   - **Min FL / Max FL** — the altitude band to meter.
   - **Scope (ARTCCs)** — limit metering to specific ARTCCs' airspace.

## Metering crossings

Once an FCA exists, OIS finds every aircraft whose **filed route** crosses the line, computes each one's ETA to the crossing, and sequences them. You pick a spacing mode:

- **Rate (aircraft/hour)** — a fixed acceptance rate, applied as even time spacing.
- **MIT (miles-in-trail)** — a distance gap, scaled by each aircraft's crossing speed.

The map draws the matched aircraft **tinted and numbered in crossing order**, and the FCA's row shows a live **count** of how many aircraft it's currently metering. When you need to override the computed order, you can **reorder** the sequence manually.

## Enabling & disabling

Each FCA has an on/off **switch** in the list. Turn one **off** and it's **hidden from the map but stays in the list** — its filters, spacing, and any releases are preserved, ready to switch back on. (A disabled FCA you've selected still draws, so you can inspect or edit it before re-enabling.)

::: tip Disable, don't delete
Use the switch to park an FCA you'll want again — a nightly arrival push, an event flow. Deleting is for FCAs you're truly done with. Only staff with `flow.fca.update` see the switch.
:::

## Releasing metered departures

For FCA-metered flights still on the ground, you issue [CFR releases](/tmu/releases) — either from the FCA map itself or, for a whole desk's worth of departures at once, from the [IDST console](/tmu/idst).

> This page will grow with step-by-step walkthroughs. The essentials above match the current builder.
