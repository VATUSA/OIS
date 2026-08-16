# The ATC layer

Toggle **ATC** in the map toolbar to overlay online controllers, VATSIM-Radar-style. It shows three things at once.

## Airport badges

Each staffed airport gets a compact stack of colored pills for its ground-level positions:

| Badge | Position |
| ----- | -------- |
| **D** | Clearance Delivery |
| **G** | Ground |
| **T** | Tower |
| **A** | ATIS |

Hover a badge to see every position at that airport with its callsign and frequency (and the ATIS letter).

## Approach areas (TRACON)

Online approach/departure controllers are drawn as shaded **TRACON polygons** from the SimAware TRACON project, labelled with the TRACON id (e.g. `N90`, `SCT`, `PCT`). When a position has no matching polygon, a circle is drawn around its airport instead.

## Center areas (ARTCC)

Online center controllers shade their **ARTCC boundary**, labelled with the center id (e.g. `ZDC`, `ZLA`).

## Reading the tooltips

Hover the **label tag** of a TRACON or center area (the `N90` / `ZDC` pill) to see the positions working it and their frequencies. Hovering the shaded area itself does nothing — only the tag opens the tooltip, so it stays out of your way.

::: info Scope
Airport badges and TRACON areas are drawn wherever controllers are online. Center shading is US ARTCCs (the boundaries OIS ships with); non-US centers aren't shaded.
:::
