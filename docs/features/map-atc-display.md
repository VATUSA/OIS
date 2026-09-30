# Map / ATC display

> **Rendering pipeline, not a domain.** This spec has no tables, permissions, API or Discord
> touchpoints of its own — it documents how the map draws ATC areas and resolves hover, which the
> template's sections do not fit. Those sections are answered explicitly at the end rather than
> omitted. The data it draws comes from the `flow` and `feed` code: the facility map itself is
> specified in [flow.md](flow.md).
>
> **Merge order:** the geometry contract below is enforced by #487 and the bundled-asset invariants
> by #485. Both target `next`; this doc should land after them.

## Problem

Six of this pipeline's defects have been "fixed" and come back: #160, #186 and #318 for stretched
wedges, #211 and #323 for ATC hover, and #477 for both at once. Every one of those fixes is still
present and intact in the code. None of them could have caught what was actually broken.

Reading the history, the reason is consistent: the rules that matter here were never written down.

- The **pick-resolution rule** — which layer wins a hover, and what looks underneath — was an
  accident of layer-push order. Nobody could see it was load-bearing, so #323's re-pick was added
  inside a conditional and the default configuration was left unable to show an ATC card at all.
- The **geometry contract** — what a ring must satisfy before it reaches a filled layer — was never
  stated, so three rounds of hardening all guarded *vertices* while the real failure was ring
  *topology*, sitting in the bundled asset the whole time.
- The **coordinate convention** differs between paths, and nothing said so.
- #482 was filed, and abandoned, on the strength of a claim that `atc-centers` draws no outline. It
  does. A doc stating so would have prevented the issue existing.

This spec exists so the next change to this pipeline starts from the rules rather than rediscovering
them.

## Scope

The ATC area/hover rendering path: the bundled ARTCC boundaries, the live TRACON rings, the ATC hover
targets, and the tooltip resolution over them. Aircraft glyphs, trails, routes, FCAs and the draft
editors appear here only where they affect pick order.

Out of scope: the facility map's aircraft colour rules ([flow.md](flow.md)), the ATC datafeed's
ingestion, and the badge/pill DOM markers themselves.

## Coordinate convention

**Three orders are in play, and the conversion points are the only safe places to be confused.**

| Where | Order | Notes |
| --- | --- | --- |
| Bundled GeoJSON (`web/src/assets/`, `backend/data/`) | `[lon, lat]` | RFC 7946. Both copies are byte-identical and asserted so. |
| deck.gl layer input | `[lon, lat]` | What every `getPosition` / `getPolygon` must produce. |
| App internals (`LatLng` in `web/src/components/map/lib/geo.ts`) | `[lat, lon]` | The TRACON feed path, `haversine`, `centroid`, the ATC anchors. |
| Backend rings (`backend/src/feed/airspace.rs`) | `[lat, lon]` | Converted from GeoJSON at parse; `Ring` is documented as `[lat, lon]`. |

So the flow is **GeoJSON `[lon, lat]` in → `[lat, lon]` internally → `[lon, lat]` back out to deck**,
and the GeoJSON layers (`atc-centers`, `artcc-boundaries`) skip the middle step entirely because they
hand deck the collection unchanged.

`toDeckPath` and `toDeckPoint` (`lib/geo.ts`) are the conversion boundary — going from internal
`[lat, lon]` to deck's `[lon, lat]`. **Anything reaching a deck layer without passing through one of
them, or without already being GeoJSON, is a bug.**

A transposed pair is usually caught by the latitude bound rather than by looking wrong:
`isValidPoint` (`layers/atc.ts`) bounds `p[0]` at 90 and `p[1]` at 180, so a `[lon, lat]` pair for
most of the US fails the latitude check. That is deliberate.

## The layer stack, and its order

`TrafficMap.tsx` pushes layers bottom-first. **Later push = drawn on top = wins the pick.** The order
is load-bearing, not cosmetic:

| # | Layer(s) | Pickable |
| --- | --- | --- |
| 1 | `artcc-boundaries` (`buildBoundaryLayer`) | no |
| 2 | `atc-centers`, `atc-tracon-polys`, `atc-tracon-circles` (`buildAtcLayers`) | no |
| 3 | trails, route overlays, named routes, rings | no |
| 4 | `fca-lines` | **yes** |
| 5 | `atc-hover` | **yes** |
| 6 | `matched` / `matched-<fcaId>` | **yes** |
| 7 | selected track, selected route | no |
| 8 | `aircraft` | **yes** |
| 9 | labels, waypoints, draft editors | no |

Four layers are pickable, one per module (`fca.ts`, `atc.ts`, `matched.ts`, `aircraft.ts`).

Two placements are deliberate and easy to undo by accident:

- **`atc-hover` sits above `fca-lines`**, so an ATC pill wins the hover card over a line running
  under it. A *click* that needs the line looks back through the pill — see below.
- **`atc-hover` sits below the aircraft glyphs.** That is intentional for the glyph's own hover, but
  it means the glyph wins the raw pick, which is why pick resolution cannot rely on order alone.

`atc-hover` is only built when map tooltips are on: an invisible pick target that draws nothing would
otherwise just eat clicks.

## Pick resolution

**deck reports only the topmost pickable layer.** Anything that needs what is underneath must ask
again, restricted to the layer it wants — that is `objectUnder(info, layerId)` in `lib/pick.ts`.

The rule, in `mapTooltip` (`lib/tooltip.ts`):

> **The smaller, more deliberate target wins the card.** An ATC pill is 13–19px and placed at an
> airport; an aircraft glyph is a 48×48 icon. When both are under the cursor the pill wins, because
> that is what someone hovering an airport badge is reaching for.

Concretely, for a pick on `aircraft` or `matched`:

1. Re-pick `atc-hover` underneath. If a pill is there, render the ATC card — **regardless of the
   aircraft-tooltip setting.**
2. Otherwise, if aircraft cards are off, no card.
3. Otherwise, the aircraft card.

Step 1 running unconditionally is the whole of #477's symptom 3. #323 added that re-pick but placed
it inside the "aircraft cards off" branch, so with default settings the glyph always won and the ATC
card could never render — while aircraft tooltips visibly worked, which is why it read as a tooltip
bug rather than a layering one. At a staffed airport there is nearly always a plane parked on the
badge, so this was not an edge case.

The reverse direction exists for clicks: `fcaLineUnder` (`lib/pick.ts`) looks through a pill for the
FCA line it covers, because the pill deliberately sits above the line.

`objectUnder` returns `null` when no deck instance is reachable from the pick — which is the case in
unit tests, so a test exercising this must supply a fake `info.layer.context.deck.pickObject`
(`pickOver` in `tooltip.test.ts` does).

## Geometry contract

**Any ring reaching a filled layer must satisfy all of the following.** A ring that fails is split
where possible and dropped otherwise — never handed to the tessellator.

1. At least three distinct vertices.
2. Every vertex finite and on the globe.
3. **No vertex visited twice.** A revisited vertex is where two loops were flattened into one ring.
4. **No two non-adjacent edges crossing.**

Rules 3 and 4 are the ones that were missing, and they are not cosmetic: earcut tessellates a ring
with two lobes joined by a zero-width bridge into triangles spanning *between* the lobes, producing
translucent wedges across the whole viewport. That is what the bundled ZNY boundary did — a single
61-vertex `Polygon` holding an oceanic lobe and the New York land lobe, both winding the same way.
It satisfied every per-vertex check for as long as it shipped.

Enforcement (all three paths, `web/src/components/map/lib/geo.ts`):

- `sanitizeRings` — the TRACON path, on `[lat, lon]` rings. Applied **in addition to**
  `isValidRing`/`MAX_RING_OUTLIER_DEG`, which catch off-globe and far-outlier vertices: a different
  class, still wanted.
- `sanitizeBoundaries` — `atc-centers` and `buildBoundaryLayer`, on GeoJSON. A `Polygon` whose ring
  splits becomes a `MultiPolygon`. Memoized on the collection; an unchanged collection is returned by
  identity so downstream memoization holds.
- `sanitize_ring` (`backend/src/feed/tracon.rs`) — the on-globe filter at ingest, before any of this.

**Splitting, not rejecting**, where a ring can be split: a genuinely multi-lobe TRACON (N90 and PCT
are both multi-lobe upstream) then draws each lobe correctly instead of vanishing. A self-crossing
ring has no shared vertex to split at, so it is rejected.

**Holes.** The web path keeps a polygon's holes only when its outer ring did not split — once it
splits there is no way to say which lobe a hole belongs to without a point-in-polygon test, and the
bundled asset has no holes at all. The backend drops holes outright, because `TraconData` carries a
flat list of rings with no hole structure; it logs what it dropped, since a donut drawn solid is
otherwise indistinguishable from correct data.

### Bundled asset invariants

`web/src/assets/artcc-boundaries.test.ts` asserts, over **both** copies: rings closed, ≥4 vertices, no
repeated interior vertex, coordinates in range, outer rings wound consistently (the asset is
clockwise throughout), `properties.id` unique, and **the two copies byte-identical**. There is no
regeneration script and no owner for this data (`source: "squawk-airspace-data"`), so that last
assertion is the only thing stopping the copies drifting.

`ZAN` is one feature with two parts because its airspace crosses the antimeridian — that split is
correct GeoJSON and must not be "fixed". `ZAK` (Oakland Oceanic) and `ZSU` (San Juan) have **no
polygon in the asset**, so a controller on either shades nothing.

## What shades, and when

`atc-centers` draws the ARTCC polygons of centres that are currently online, filtered by matching each
**feature's** `properties.id` against the online set. Consequences worth knowing:

- An online id with no matching feature draws nothing. An unknown or misspelled centre id therefore
  cannot select a polygon — the filter is feature-driven, not id-driven.
- `center_artcc` (`backend/src/handlers/atc.rs`) maps FAA radio prefixes to `Zxx` ids, and accepts a
  bare `Zxx` verbatim. `"BDA" | "NY" => "ZNY"` is **correct**: New York Oceanic (KZWY) is
  ZNY-controlled. `"ZMO" => "ZMA"` is unverified — see Open questions.
- **Every shaded centre is also outlined**, by `atc-centers` itself (`stroked: true`, the ATC centre
  colour at alpha 140, 1.5px). Shading and outline do come from different props — `boundaries` is the
  selected facility, `atcBoundaries` the national set — but nothing is ever shaded without a stroke.
  Centre shading is nationwide on purpose (`51cbb4f`), matching airport badges and TRACONs, which
  already drew nationwide.

## Data model, Permissions, API, Discord

**None of its own.** This is client-side rendering:

- **Data model** — reads `tmu`/`flow`/`feed` state and the bundled boundary asset; owns no table.
- **Permissions** — none. The facility map is public; the ATC overlay is a client toggle.
- **API** — consumes the ATC and TRACON feed endpoints; adds none.
- **Discord** — none.

## Open questions

- **`"ZMO" => "ZMA"`** in `center_artcc` maps something to Miami Center and nobody has established
  what `ZMO` is. Settling it needs VATSpy's `[FIRs]` list, which is not in this repo. It is harmless
  unless `ZMO` is a non-US FIR, in which case a controller there shades Miami.
- **Nobody owns `artcc-boundaries.json`.** There is no regeneration script and no recorded
  provenance beyond `source: "squawk-airspace-data"`. The invariant test keeps it honest but cannot
  refresh it.
- **Collinear overlap is not treated as self-crossing** by the topology check — two edges lying along
  each other enclose no area, so they are left alone. If a real ring is ever found that wedges this
  way, that is the assumption to revisit.
