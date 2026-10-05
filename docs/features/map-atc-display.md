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

## Status

This document specifies the pipeline **as of #477, #481 and #482 together**, and parts of it
describe code that arrives with those: `sanitizeRings` / `sanitizeBoundaries` (#481), the
`artcc-boundaries.test.ts` invariants (#477), and `Boundaries::has` (#482). It should land after
them — merged first, it describes a pipeline that does not exist yet.

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
| 9 | labels, waypoints, `draft-vertices` (`buildDraftLayers`) | `draft-vertices` **yes**, rest no |

Five layers are pickable, one per module (`fca.ts`, `atc.ts`, `matched.ts`, `aircraft.ts`,
`draft.ts`).

`draft-vertices` is the one that is easy to forget: it exists only while a draft is open
(`TrafficMap.tsx`: `if (draft) out.push(...buildDraftLayers(draft, palette))`), so it is absent from
the stack most of the time. While it is there it sits above everything below it and wins the pick,
which is the point of a vertex handle — but it means "the topmost pickable layer" is a different
layer during drafting, and any reasoning about pick order has to allow for it.

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
assertion is the only thing stopping the copies drifting. (This is the ARTCC/TRACON *boundary* asset only.
The altitude-bounded *sector* volumes are a different dataset, with an owner and an importer:
`airspace-sector-importer`, see [`monitor.md`](monitor.md#the-sector-dataset).)

`ZAN` is one feature with two parts because its airspace crosses the antimeridian — that split is
correct GeoJSON and must not be "fixed". `ZAK` (Oakland Oceanic) and `ZSU` (San Juan) have **no
polygon in the asset**, so a controller on either shades nothing.

## What shades, and when

`atc-centers` draws the ARTCC polygons of centres that are currently online, filtered by matching each
**feature's** `properties.id` against the online set. Consequences worth knowing:

- An online id with no matching feature draws nothing. An unknown or misspelled centre id therefore
  cannot select a polygon — the filter is feature-driven, not id-driven. This is what makes the
  client safe on its own terms, whatever the API sends.
- `center_artcc` (`backend/src/handlers/atc.rs`) maps FAA radio prefixes to `Zxx` ids, and accepts a
  bare `Zxx` verbatim. It answers **"which US ARTCC is this?"** and nothing else — deliberately not
  "can we draw it", because its other caller (`feed::stats::is_us_controller`) uses it to decide
  whether a controller counts as American, and `ZAK`/`ZSU` must keep counting despite having no
  polygon.
- **`BDA` is not mapped.** Bermuda (TXKF) is not a VATUSA position, so a `BDA_CTR` is neither shaded
  nor counted as a US controller (#482, settled with the repo owner). An earlier reading had it as a
  ZNY position on the grounds that New York Oceanic is ZNY-controlled; that conflated the two, and the
  alias meant one Bermuda controller shaded the whole of New York.
- **A centre with no polygon never reaches the board.** `board_from` filters on
  `Boundaries::has` at its own call site (#482), so the id list the client receives is already
  restricted to centres the bundled asset can draw. That filter is *in addition to* the
  feature-driven client filter above, not a replacement for it — each is independently sufficient to
  stop an unknown id shading anything, and the backend one also keeps the board from advertising a
  centre the map would silently ignore.
- **Every shaded centre is also outlined**, by `atc-centers` itself (`stroked: true`, the ATC centre
  colour at alpha 140, 1.5px). Shading and outline do come from different props — `boundaries` is the
  selected facility, `atcBoundaries` the national set — but nothing is ever shaded without a stroke.
  Centre shading is nationwide on purpose (`51cbb4f`), matching airport badges and TRACONs, which
  already drew nationwide.

## Data model, Permissions, API, Discord

**The ATC overlay has none of its own.** It is client-side rendering:

- **Data model** — reads `tmu`/`flow`/`feed` state and the bundled boundary asset; the overlay owns no
  table.
- **Permissions** — none. The facility map is public; the ATC overlay is a client toggle.
- **API** — consumes the ATC and TRACON feed endpoints; adds none.
- **Discord** — none.

**ATC _sector_ data is not part of this pipeline, and has its own table, permission and API.**
Altitude-bounded sector volumes live in `flow.airspace_sector`, and the admin sector viewer draws them
from `GET /api/v1/flow/airspace/sectors`, gated by `flow.sectors.read`. See [`monitor.md`](monitor.md).
(The Airspace Monitor built on them, with its alert parameters and consolidations, was removed in #719.)

## Open questions

- **What `ZMO` is** — nobody has established it, and settling it needs VATSpy's `[FIRs]` list, which
  is not in this repo. `center_artcc` used to carry a `"ZMO" => "ZMA"` arm, removed in #482 because it
  could never fire: the bare-`Zxx` branch at the top of the function returns any three-character
  alphanumeric `Z`-prefixed id verbatim, so `ZMO` resolved to `ZMO` and the table below was never
  reached. The open question is therefore **not** that a `ZMO` controller might shade Miami — that was
  never reachable — but that a non-US FIR whose id happens to look like a US ARTCC id resolves as US.
  Today it is inert for the map, because such an id has no polygon and #482's filter drops it before
  the board; it still counts that controller as American in `feed::stats::is_us_controller`, which has
  no geometry to filter on.
- **Nobody owns `artcc-boundaries.json`.** There is no regeneration script and no recorded
  provenance beyond `source: "squawk-airspace-data"`. The invariant test keeps it honest but cannot
  refresh it.
- **Collinear overlap is not treated as self-crossing** by the topology check — two edges lying along
  each other enclose no area, so they are left alone. If a real ring is ever found that wedges this
  way, that is the assumption to revisit.
