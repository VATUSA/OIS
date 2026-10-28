# Flow Constrained Areas

A **Flow Constrained Area** (FCA) is a **line you draw across a flow** to identify and meter the traffic crossing it — the building block for most flow initiatives in OIS. Aircraft are metered as their filed route **crosses the line**, so an FCA is an open boundary, not a closed area.

![The FCA map: two FCAs in the list with their enable toggles and live crossing counts, and FCA 1 drawn as a dashed line across the flow.](/screenshots/fca-map.png)

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

### Manual order and late joiners

Drag a strip in the FCA panel to put the sequence in your own order; **manual** in the panel header returns it to automatic sequencing. Each aircraft is spaced behind the one you put ahead of it, so moving one up delays everyone behind it.

An aircraft that starts crossing the line after you set the order is merged in by its ETA: it goes directly behind the last ordered aircraft whose ETA is at or before its own. It never goes ahead of an ordered aircraft that would reach the line first, and it never moves an aircraft you placed.

### Why one aircraft is ahead of another

Each strip shows its **gap** to the aircraft ahead, in nm and minutes:seconds, at the aircraft's predicted crossing speed. A gap equal to the required spacing means the aircraft is being held back to keep that spacing.

Press **Compare** on two strips to open the compare panel. It shows each aircraft's crossing time, crossing speed and distance to the line, the gap between them, the spacing required behind the one ahead, and one line of reason:

- **Manual placement** — you put them in this order, or one of them is in your order and the other was merged around it.
- **ETA order** — the one ahead simply reaches the line first.
- **Spacing push** — the one behind would reach the line first, but was pushed behind an airborne aircraft or an issued release to hold spacing.

## Enabling & disabling

Each FCA has an on/off **switch** in the list. Turn one **off** and it's **hidden from the map but stays in the list** — its filters, spacing, and any releases are preserved, ready to switch back on. (A disabled FCA you've selected still draws, so you can inspect or edit it before re-enabling.)

::: tip Disable, don't delete
Use the switch to park an FCA you'll want again — a nightly arrival push, an event flow. Deleting is for FCAs you're truly done with. Only staff with `flow.fca.update` see the switch.
:::

## Releasing metered departures

For FCA-metered flights still on the ground, you issue [CFR releases](/tmu/releases) — either from the FCA map itself or, for a whole desk's worth of departures at once, from the [IDST console](/tmu/idst).
