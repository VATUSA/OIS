import {describe, expect, it} from "vitest";

import {
  centroid,
  haversine,
  lineNm,
  midpointOf,
  normalizeLng,
  sanitizeBoundaries,
  sanitizeRingTopology,
  sanitizeRings,
  toDeckPath,
  toDeckPoint,
} from "./geo";
import boundariesGeo from "@/assets/artcc-boundaries.json";

describe("haversine", () => {
  it("is zero for an identical point", () => {
    expect(haversine([40, -80], [40, -80])).toBe(0);
  });

  it("matches a known great-circle distance (~60nm per degree of latitude)", () => {
    const nm = haversine([0, 0], [1, 0]);
    expect(nm).toBeCloseTo(60.04, 1);
  });
});

describe("lineNm", () => {
  it("sums consecutive segment lengths", () => {
    const total = lineNm([
      [0, 0],
      [1, 0],
      [1, 1],
    ]);
    expect(total).toBeCloseTo(haversine([0, 0], [1, 0]) + haversine([1, 0], [1, 1]), 6);
  });

  it("is zero for a single point", () => {
    expect(lineNm([[0, 0]])).toBe(0);
  });
});

describe("midpointOf", () => {
  it("returns the exact midpoint of a straight two-point line", () => {
    const [lat, lon] = midpointOf([
      [0, 0],
      [2, 0],
    ]);
    expect(lat).toBeCloseTo(1, 5);
    expect(lon).toBeCloseTo(0, 5);
  });

  it("falls on the correct segment for an unevenly-spaced three-point line", () => {
    // A long first leg then a short second leg — the arc-length midpoint should land partway
    // through the first (longer) segment, not at the shared vertex.
    const pts: [number, number][] = [
      [0, 0],
      [10, 0],
      [10, 1],
    ];
    const [lat, lon] = midpointOf(pts);
    expect(lat).toBeGreaterThan(0);
    expect(lat).toBeLessThan(10);
    expect(lon).toBe(0);
  });

  it("returns the single point for a degenerate one-point input", () => {
    expect(midpointOf([[5, 5]])).toEqual([5, 5]);
  });
});

describe("normalizeLng", () => {
  it("leaves an in-range longitude untouched", () => {
    expect(normalizeLng(45)).toBe(45);
    expect(normalizeLng(-179)).toBe(-179);
  });

  it("wraps a longitude past 180", () => {
    expect(normalizeLng(190)).toBeCloseTo(-170, 6);
  });

  it("wraps a longitude past -180", () => {
    expect(normalizeLng(-190)).toBeCloseTo(170, 6);
  });

  it("maps exactly 180 into the [-180, 180) range", () => {
    expect(normalizeLng(180)).toBeCloseTo(-180, 6);
  });
});

describe("centroid", () => {
  it("averages a ring's points", () => {
    const [lat, lon] = centroid([
      [0, 0],
      [2, 0],
      [1, 3],
    ]);
    expect(lat).toBeCloseTo(1, 6);
    expect(lon).toBeCloseTo(1, 6);
  });
});

describe("toDeckPath / toDeckPoint", () => {
  it("swaps [lat, lon] to [lon, lat] for a path", () => {
    expect(
      toDeckPath([
        [40, -80],
        [41, -79],
      ]),
    ).toEqual([
      [-80, 40],
      [-79, 41],
    ]);
  });

  it("returns an empty path for an empty input", () => {
    expect(toDeckPath([])).toEqual([]);
  });

  it("swaps [lat, lon] to [lon, lat] for a single point", () => {
    expect(toDeckPoint([40, -80])).toEqual([-80, 40]);
  });
});

// --- ring topology (VATUSA/OIS#481) --------------------------------------------------------------

/** Twice the signed area of a closed ring; the sign is the winding. */
function signedArea(ring: number[][]): number {
  let sum = 0;
  for (let i = 0; i < ring.length - 1; i++) sum += ring[i][0] * ring[i + 1][1] - ring[i + 1][0] * ring[i][1];
  return sum / 2;
}
const closed = (ring: number[][]) => [...ring, ring[0]];
const repeatsIn = (ring: number[][]) => {
  const seen = new Set<string>();
  return ring.some((p) => {
    const k = `${p[0]},${p[1]}`;
    if (seen.has(k)) return true;
    seen.add(k);
    return false;
  });
};

const fc = boundariesGeo as unknown as GeoJSON.FeatureCollection;
const ringOf = (id: string) => {
  const f = fc.features.find((x) => x.properties?.id === id);
  const g = f!.geometry as GeoJSON.Polygon | GeoJSON.MultiPolygon;
  return (g.type === "Polygon" ? g.coordinates[0] : g.coordinates[0][0]) as number[][];
};

describe("sanitizeRingTopology", () => {
  // Driven by the real bundled ZNY ring rather than a synthetic bowtie, so the test cannot drift from
  // the data that actually broke. On this branch the asset is still the pre-#485 Polygon; once that
  // merges, ZNY is a MultiPolygon and each lobe passes through whole, which this still asserts.
  const zny = ringOf("ZNY");

  it("splits the real ZNY ring into exactly its two lobes", () => {
    const verdict = sanitizeRingTopology(zny as [number, number][]);
    expect(verdict.ok).toBe(true);
    if (!verdict.ok) return;

    // Asserted as properties, not a hand-derived vertex list: any correct decomposition should pass,
    // and a fixed list would be asserting my arithmetic rather than the behaviour.
    const expected = zny.length > 40 ? 2 : 1; // 61-vertex bowtie pre-#485; one clean lobe after
    expect(verdict.rings).toHaveLength(expected);
    for (const ring of verdict.rings) {
      expect(ring.length).toBeGreaterThanOrEqual(3);
      expect(repeatsIn(ring)).toBe(false);
    }
    // The split encloses the same area as the original: nothing invented, nothing lost. The discarded
    // bridge remnants are zero-area by construction.
    const total = verdict.rings.reduce((sum, r) => sum + signedArea(closed(r)), 0);
    expect(total).toBeCloseTo(signedArea(closed(zny)), 6);
  });

  it("leaves a well-formed ring alone", () => {
    const square: [number, number][] = [
      [0, 0],
      [0, 1],
      [1, 1],
      [1, 0],
    ];
    const verdict = sanitizeRingTopology(square);
    expect(verdict.ok).toBe(true);
    if (verdict.ok) expect(verdict.rings).toEqual([square]);
  });

  it("rejects a ring whose edges cross without sharing a vertex", () => {
    // A true bowtie: no repeated vertex to split at, so there is nothing to salvage.
    const crossing: [number, number][] = [
      [0, 0],
      [2, 2],
      [0, 2],
      [2, 0],
    ];
    const verdict = sanitizeRingTopology(crossing);
    expect(verdict.ok).toBe(false);
    if (!verdict.ok) expect(verdict.reason).toContain("cross");
  });

  it("rejects a ring with too few distinct vertices", () => {
    expect(sanitizeRingTopology([[0, 0], [1, 1]]).ok).toBe(false);
    // Closed but degenerate — the closing repeat is not a third vertex.
    expect(sanitizeRingTopology([[0, 0], [1, 1], [0, 0]]).ok).toBe(false);
  });
});

describe("sanitizeRings", () => {
  it("keeps sound rings and drops what cannot be repaired", () => {
    const sound: [number, number][] = [
      [40, -80],
      [41, -80],
      [41, -79],
    ];
    const crossing: [number, number][] = [
      [0, 0],
      [2, 2],
      [0, 2],
      [2, 0],
    ];
    expect(sanitizeRings([sound, crossing])).toEqual([sound]);
  });

  it("turns one bridged ring into several", () => {
    // Two triangles sharing a single vertex — the shape a multi-lobe TRACON arrives as.
    const bridged: [number, number][] = [
      [0, 0],
      [1, 0],
      [1, 1],
      [0, 0],
      [5, 5],
      [6, 5],
      [6, 6],
    ];
    expect(sanitizeRings([bridged])).toHaveLength(2);
  });
});

describe("sanitizeBoundaries", () => {
  it("makes the bundled collection sound without losing a facility", () => {
    const out = sanitizeBoundaries(fc);
    const ids = (c: GeoJSON.FeatureCollection) =>
      [...new Set(c.features.map((f) => String(f.properties?.id)))].sort();
    // Every ARTCC still present: this repairs geometry, it does not discard facilities.
    expect(ids(out)).toEqual(ids(fc));
    for (const f of out.features) {
      const g = f.geometry as GeoJSON.Polygon | GeoJSON.MultiPolygon;
      const polys = g.type === "Polygon" ? [g.coordinates] : g.coordinates;
      for (const poly of polys) expect(repeatsIn((poly[0] as number[][]).slice(0, -1))).toBe(false);
    }
  });

  it("promotes a split Polygon to a MultiPolygon", () => {
    const bridged: GeoJSON.FeatureCollection = {
      type: "FeatureCollection",
      features: [
        {
          type: "Feature",
          properties: {id: "TEST"},
          geometry: {
            type: "Polygon",
            coordinates: [[[0, 0], [1, 0], [1, 1], [0, 0], [5, 5], [6, 5], [6, 6], [0, 0]]],
          },
        } as GeoJSON.Feature,
      ],
    };
    const out = sanitizeBoundaries(bridged);
    const g = out.features[0].geometry as GeoJSON.MultiPolygon;
    expect(g.type).toBe("MultiPolygon");
    expect(g.coordinates).toHaveLength(2);
    // Closed back up, as GeoJSON requires.
    for (const poly of g.coordinates) expect(poly[0][0]).toEqual(poly[0][poly[0].length - 1]);
  });

  it("returns an already-sound collection by identity, so downstream memoization still works", () => {
    const sound: GeoJSON.FeatureCollection = {
      type: "FeatureCollection",
      features: [
        {
          type: "Feature",
          properties: {id: "OK"},
          geometry: {type: "Polygon", coordinates: [[[0, 0], [0, 1], [1, 1], [0, 0]]]},
        } as GeoJSON.Feature,
      ],
    };
    expect(sanitizeBoundaries(sound)).toBe(sound);
  });

  it("keeps a hole when the outer ring stays whole", () => {
    const donut: GeoJSON.FeatureCollection = {
      type: "FeatureCollection",
      features: [
        {
          type: "Feature",
          properties: {id: "DONUT"},
          geometry: {
            type: "Polygon",
            coordinates: [
              [[0, 0], [0, 10], [10, 10], [10, 0], [0, 0]],
              [[2, 2], [2, 4], [4, 4], [2, 2]],
            ],
          },
        } as GeoJSON.Feature,
      ],
    };
    const g = sanitizeBoundaries(donut).features[0].geometry as GeoJSON.Polygon;
    expect(g.coordinates).toHaveLength(2);
  });
});
