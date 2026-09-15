# Runway Balancer

The **Runway Balancer** is the shared arrival→runway picture for one airport: which runways are
active, which STAR feeds which runway, and how demand is spreading across them — the same board
for every controller working the field.

![The Runway Balancer for KJFK: active runway toggles and presets on the left, a 10-min demand histogram and arrivals grouped by runway on the right.](/screenshots/tmu-runway-balancer.png)

## Active runways

Toggle a runway end on or off, or apply a **flow preset** (W/E/N/S) that activates every end within
65° of that heading in one click — a fast way to switch configuration during a runway change.

## STAR → runway rules

Assign a rule mapping a STAR (by its base name, e.g. `JJEDI` from `JJEDI4`) to a runway. Every
arrival on that STAR is recommended that runway; arrivals with no matching rule (or on an inactive
runway) show as **unassigned**. Add or remove rules any time — they apply to the live board
immediately for everyone.

## Arrivals by runway

Live arrivals are grouped into a card per active runway end (plus an **Unassigned** group), each
showing its heading and a live count. A recommendation engine suggests a runway per arrival from
the STAR rules; you can override any flight's assigned runway by hand, which the whole board sees.

## Demand board

A configurable-width bucketed histogram (the window length is adjustable) shows arrival counts
building over the near term, colored by how close to (or over) capacity each bucket is — the same
"spot a wave before it hits" idea as the [arrival demand chart](/tmu/aadc), but scoped to runway
assignment rather than a carrier/fix/category breakdown.

## Saved configurations

Save the current active-runway set + STAR rules as a **named config** (e.g. "West flow", "Night
ops") and reapply it later in one click instead of rebuilding it by hand.

::: info Permissions
Viewing requires `flow.runway.read`. Toggling runways, editing STAR rules, and overriding an
arrival's runway all require `flow.runway.update`.
:::
