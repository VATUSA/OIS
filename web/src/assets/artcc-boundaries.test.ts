import {readFileSync} from "node:fs";

import {describe, expect, it} from "vitest";

/**
 * VATUSA/OIS#477 — the bundled ARTCC boundaries had never been checked for anything.
 *
 * Three rounds of wedge fixes (#160, #186, #318) all hardened individual *vertices*, and every one of
 * them is still present and intact. None could catch what was actually wrong: ZNY shipped as a single
 * `Polygon` whose one ring held two same-winding lobes joined by a zero-width bridge, which earcut
 * tessellates into triangles spanning between the lobes — the map-wide wedges. A ring can have every
 * vertex finite, in range and near its neighbours and still be topologically broken.
 *
 * So this asserts ring *topology*, and it asserts it over **both copies**: `web/src/assets` (bundled
 * into the web app) and `backend/data` (embedded into the binary by `feed::airspace` via
 * `include_str!`). There is no regeneration script for either and no owner, so the copies staying
 * identical is itself something worth pinning.
 */

const WEB = new URL("./artcc-boundaries.json", import.meta.url);
const BACKEND = new URL("../../../backend/data/artcc-boundaries.json", import.meta.url);

type Ring = number[][];
interface Feature {
  properties?: {id?: string};
  geometry: {type: string; coordinates: Ring[] | Ring[][]};
}

const readRaw = (url: URL) => readFileSync(url, "utf8");
const parse = (raw: string) => JSON.parse(raw) as {features: Feature[]};

/** Every polygon of a feature, as a list of rings (ring 0 outer, the rest holes). */
function polygonsOf(f: Feature): Ring[][] {
  // A Polygon's `coordinates` is already a list of rings (outer first, then holes), so it becomes a
  // one-element list of polygons; a MultiPolygon's is a list of those.
  return f.geometry.type === "Polygon"
    ? [f.geometry.coordinates as Ring[]]
    : (f.geometry.coordinates as Ring[][]);
}

/** Twice the signed area; its sign is the winding. `[lon, lat]` order, as the asset stores it. */
function signedArea(ring: Ring): number {
  let sum = 0;
  for (let i = 0; i < ring.length - 1; i++) {
    sum += ring[i][0] * ring[i + 1][1] - ring[i + 1][0] * ring[i][1];
  }
  return sum / 2;
}

/** Vertices visited more than once, ignoring the closing repeat of the first. */
function repeatedInterior(ring: Ring): string[] {
  const seen = new Map<string, number>();
  for (const p of ring.slice(0, -1)) {
    const k = `${p[0]},${p[1]}`;
    seen.set(k, (seen.get(k) ?? 0) + 1);
  }
  return [...seen.entries()].filter(([, n]) => n > 1).map(([k]) => k);
}

const label = (f: Feature, part: number, ringIdx: number) =>
  `${f.properties?.id ?? "(no id)"} part ${part} ring ${ringIdx}`;

describe.each([
  ["web/src/assets", WEB],
  ["backend/data", BACKEND],
])("%s/artcc-boundaries.json is well-formed", (_name, url) => {
  const {features} = parse(readRaw(url));

  it("has features to check", () => {
    // Without this a file that failed to load, or an empty FeatureCollection, would make every
    // assertion below vacuous and this suite would pass while checking nothing.
    expect(features.length).toBeGreaterThan(20);
  });

  it("gives every feature a unique properties.id", () => {
    const ids = features.map((f) => f.properties?.id);
    expect(ids.every((id) => typeof id === "string" && id.length > 0)).toBe(true);
    // A shared id is not harmless: `facilityFeature()` uses `.find()` and renders one, while
    // `atc-centers` uses `.filter()` and shades both.
    expect([...new Set(ids)]).toHaveLength(ids.length);
  });

  it("closes every ring, with at least four vertices", () => {
    for (const f of features) {
      polygonsOf(f).forEach((poly, part) =>
        poly.forEach((ring, ringIdx) => {
          expect(ring.length, `${label(f, part, ringIdx)} is too short to enclose anything`)
            .toBeGreaterThanOrEqual(4);
          expect(ring[0], `${label(f, part, ringIdx)} is not closed`).toEqual(ring[ring.length - 1]);
        }),
      );
    }
  });

  it("visits no vertex twice within a ring", () => {
    // The ZNY failure mode: a repeated interior vertex is where two lobes were bridged into one ring,
    // and it is what earcut turns into wedges. Vertex-level checks cannot see it.
    for (const f of features) {
      polygonsOf(f).forEach((poly, part) =>
        poly.forEach((ring, ringIdx) => {
          expect(repeatedInterior(ring), `${label(f, part, ringIdx)} revisits a vertex`).toEqual([]);
        }),
      );
    }
  });

  it("keeps every coordinate on the globe", () => {
    for (const f of features) {
      polygonsOf(f).forEach((poly, part) =>
        poly.forEach((ring, ringIdx) => {
          for (const [lon, lat] of ring) {
            expect(Number.isFinite(lon) && Number.isFinite(lat), label(f, part, ringIdx)).toBe(true);
            expect(Math.abs(lon), `${label(f, part, ringIdx)} lon out of range`).toBeLessThanOrEqual(180);
            expect(Math.abs(lat), `${label(f, part, ringIdx)} lat out of range`).toBeLessThanOrEqual(90);
          }
        }),
      );
    }
  });

  it("winds every outer ring the same way", () => {
    // Holes legitimately wind the other way, so only outer rings are compared. The asset is
    // clockwise throughout; one ring winding the other way is a sign it came from a different
    // source or was hand-edited, and it makes any future orientation-sensitive check unreliable.
    const wrong: string[] = [];
    const signs = features.flatMap((f) =>
      polygonsOf(f).map((poly, part) => ({name: label(f, part, 0), sign: Math.sign(signedArea(poly[0]))})),
    );
    const majority = Math.sign(signs.filter((s) => s.sign < 0).length >= signs.length / 2 ? -1 : 1);
    for (const s of signs) if (s.sign !== majority) wrong.push(s.name);
    expect(wrong, "these outer rings wind against the rest of the asset").toEqual([]);
  });
});

it("keeps the web and backend copies identical", () => {
  // Two byte-identical copies with no regeneration script: the only thing stopping them drifting is
  // this assertion. `feed::airspace` embeds the backend one with `include_str!`, so a fix applied to
  // only one copy would leave the map and the FCA scope filter disagreeing about US airspace.
  expect(readRaw(WEB)).toBe(readRaw(BACKEND));
});
