# The facility map

The **facility map** is a public, per-facility traffic picture. Open it for any ARTCC and you get every online VATSIM aircraft, centered on that facility's airspace and colored by the facility's own rules — no sign-in required.

It lives at **`/facility-map`** (a national overview) and **`/facility-map/ZDC`** (a single facility). Anyone can view it; only staff with the right permission can change how a facility colors its traffic.

![The facility map for Los Angeles Center (ZLA), showing live VATSIM traffic over the facility's boundary.](/screenshots/facility-map-zla.png)

## Opening a facility

- Open **`/facility-map`** for the national overview — every ARTCC boundary, faint, framed on the CONUS.
- Pick a facility from the **selector** in the top-left (or go straight to `/facility-map/<id>`, e.g. `/facility-map/ZLA`).
- The map **re-frames** on the facility you pick and emphasizes its boundary. **Recenter** puts you back on the facility (or the CONUS on the overview) any time.

::: tip
The picker only lists ARTCCs that have both a VATUSA directory entry and a boundary on file. If a URL names a facility with no boundary, the map tells you and keeps the picker open.
:::

## Coloring the traffic

Each facility can color aircraft by what matters to *them* — arrivals to a particular field, a specific STAR, heavies, high-altitude overflights, and so on. When a facility has color rules, a **legend** appears in the bottom-left naming each color.

The traffic itself is the same live VATSIM feed you see on the [flow map](/map/overview); the facility map just recolors it.

## Editing color rules

If you hold **`flow.facility_map.update`** scoped to the facility, an **Edit rules** button appears. It opens a side panel where you build an ordered list of rules:

- **A rule** has a name, a color (from the palette), and one or more **conditions**. An aircraft takes the color of the **first enabled rule whose conditions all match**; anything unmatched gets the **default (unmatched)** color.
- **Conditions** match on a flight attribute:

  | Field | Matches on | Operators |
  | ----- | ---------- | --------- |
  | Arrival airport | filed destination | is / starts with |
  | STAR / arrival gate | arrival gate parsed from the route | is / starts with |
  | Departure airport | filed origin | is / starts with |
  | Aircraft type | ICAO type | is / starts with |
  | Wake category | wake turbulence class | is / starts with |
  | Flight rules | IFR / VFR | is / starts with |
  | Filed altitude (ft) | filed cruise altitude | below / above / between |

  Text conditions take a comma-separated list (e.g. `KIAD, KBWI`); “starts with” takes a prefix (e.g. `K`); the numeric altitude field takes one value, or two for **between**.

- **Order matters** — drag rules up and down with the arrows; the first match wins. Reorder them so the most specific rules sit above the broad ones.
- The map **recolors live** as you edit, so you can see the effect before committing. **Save** writes the config for everyone; **Reset** discards your unsaved changes.

::: warning
A saved config is the facility's shared coloring — every public viewer sees it. It's scoped to your facility: `flow.facility_map.update` for one ARTCC only lets you recolor that ARTCC's map.
:::

## See also

- [Live traffic](/map/traffic) — how aircraft are drawn and what the icons mean.
- [The flow map](/map/overview) — the full controller map with FCAs and the ATC layer.
- [Roles & permissions](/reference/permissions) — how facility-scoped grants like `flow.facility_map.update` work.
