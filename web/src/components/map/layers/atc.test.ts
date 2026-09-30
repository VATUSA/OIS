import {describe, expect, it} from "vitest";

import {buildAtcHoverLayer, buildAtcLayers, computeAtcAnchors, type AtcAnchor, type AtcData} from "./atc";

const emptyBoundaries: GeoJSON.FeatureCollection = { type: "FeatureCollection", features: [] };

const validRing: number[][] = [
  [40, -80],
  [41, -80],
  [41, -79],
];

const baseAtc = (): AtcData => ({
  airports: [],
  centers: [],
  tracons: [],
});

function tracon(over: Partial<AtcData["tracons"][number]>): AtcData["tracons"][number] {
  return {
    id: "T1",
    rings: [],
    positions: [],
    ...over,
  };
}

function tracons(layers: ReturnType<typeof buildAtcLayers>) {
  return layers.find((l) => l.id === "atc-tracon-polys");
}
function circles(layers: ReturnType<typeof buildAtcLayers>) {
  return layers.find((l) => l.id === "atc-tracon-circles");
}

describe("buildAtcLayers", () => {
  it("renders a normal TRACON ring", () => {
    const atc = { ...baseAtc(), tracons: [tracon({ rings: [validRing] })] };
    const polys = tracons(buildAtcLayers(atc, emptyBoundaries));
    expect(polys).toBeDefined();
    expect((polys!.props.data as unknown[]).length).toBe(1);
  });

  it("drops a degenerate (2-point) ring but keeps a valid sibling ring on the same TRACON", () => {
    const degenerate = [
      [40, -80],
      [41, -80],
    ];
    const atc = {
      ...baseAtc(),
      tracons: [tracon({ rings: [degenerate, validRing] })],
    };
    const polys = tracons(buildAtcLayers(atc, emptyBoundaries));
    expect((polys!.props.data as unknown[]).length).toBe(1);
  });

  it("drops a ring containing a non-finite coordinate", () => {
    const withNaN: number[][] = [
      [40, -80],
      [Number.NaN, -80],
      [41, -79],
    ];
    const atc = { ...baseAtc(), tracons: [tracon({ rings: [withNaN] })] };
    const layers = buildAtcLayers(atc, emptyBoundaries);
    expect(tracons(layers)).toBeUndefined();
  });

  // VATUSA/OIS#318: finite-but-bad vertices must not spike the fill.
  it("drops a ring with an out-of-range vertex", () => {
    const outOfRange: number[][] = [...validRing, [200, -80]];
    const atc = { ...baseAtc(), tracons: [tracon({ rings: [outOfRange] })] };
    expect(tracons(buildAtcLayers(atc, emptyBoundaries))).toBeUndefined();
  });

  it("drops a ring with a transposed [lon, lat] vertex", () => {
    const transposed: number[][] = [...validRing, [-104.7, 39.9]];
    const atc = { ...baseAtc(), tracons: [tracon({ rings: [transposed] })] };
    expect(tracons(buildAtcLayers(atc, emptyBoundaries))).toBeUndefined();
  });

  it("drops a ring with one far outlier vertex but keeps a valid sibling ring", () => {
    // A stray in-range vertex ~10° away (the Gulf) turns the fill into a map-spanning wedge.
    const outlier: number[][] = [[40, -86], [40.5, -86], [29.5, -90], [40.5, -85.5], [40, -85.5]];
    const atc = { ...baseAtc(), tracons: [tracon({ rings: [outlier, validRing] })] };
    expect((tracons(buildAtcLayers(atc, emptyBoundaries))!.props.data as unknown[]).length).toBe(1);
  });

  it("falls back to a circle at the label when every ring is invalid", () => {
    const outlier: number[][] = [[40, -86], [40.5, -86], [29.5, -90]];
    const atc = { ...baseAtc(), tracons: [tracon({ rings: [outlier], label: [40.2, -86] })] };
    const layers = buildAtcLayers(atc, emptyBoundaries);
    expect(tracons(layers)).toBeUndefined();
    expect((circles(layers)!.props.data as { pos: number[] }[])[0].pos).toEqual([-86, 40.2]);
  });

  it("without a label, falls back to a circle at the dropped ring's median vertex", () => {
    const outlier: number[][] = [[40, -86], [40.5, -86], [29.5, -90], [40.5, -85.5], [40, -85.5]];
    const atc = { ...baseAtc(), tracons: [tracon({ rings: [outlier] })] };
    expect((circles(buildAtcLayers(atc, emptyBoundaries))!.props.data as { pos: number[] }[])[0].pos).toEqual([-86, 40]);
  });

  it("excludes a TRACON with no circle (empty array) from the circle-fallback layer", () => {
    const atc = {
      ...baseAtc(),
      tracons: [tracon({ circle: [] }), tracon({ id: "T2", circle: [40, -80] })],
    };
    const circleLayer = circles(buildAtcLayers(atc, emptyBoundaries));
    expect((circleLayer!.props.data as unknown[]).length).toBe(1);
  });
});

describe("computeAtcAnchors", () => {
  it("skips a TRACON whose label, circle, and rings are all invalid", () => {
    const atc = {
      ...baseAtc(),
      tracons: [tracon({ label: [], circle: [], rings: [] })],
    };
    expect(computeAtcAnchors(atc, emptyBoundaries)).toEqual([]);
  });

  it("falls through to a valid circle when the label is invalid", () => {
    const atc = {
      ...baseAtc(),
      tracons: [tracon({ label: [], circle: [40, -80] })],
    };
    const anchors = computeAtcAnchors(atc, emptyBoundaries);
    expect(anchors).toEqual([
      expect.objectContaining({ type: "area", id: "T1", lat: 40, lon: -80 }),
    ]);
  });

  it("uses the ring centroid when both label and circle are absent", () => {
    const atc = {
      ...baseAtc(),
      tracons: [tracon({ rings: [validRing] })],
    };
    const anchors = computeAtcAnchors(atc, emptyBoundaries);
    expect(anchors).toHaveLength(1);
    expect(anchors[0].lat).toBeCloseTo((40 + 41 + 41) / 3);
    expect(anchors[0].lon).toBeCloseTo((-80 + -80 + -79) / 3);
  });

  // #323: `AtcBadge` draws nothing for an airport without a DEL/GND/TWR/ATIS position, so an anchor
  // there would be an invisible hover target over empty map.
  it("anchors only airports that draw a pill", () => {
    const pos = (kind: string) => ({ callsign: `X_${kind}`, frequency: "118.000", kind, name: "", rating: 3, logon_time: "" });
    const atc = {
      ...baseAtc(),
      airports: [
        { icao: "KAPP", lat: 40, lon: -80, positions: [pos("APP")] },
        { icao: "KTWR", lat: 41, lon: -79, positions: [pos("TWR"), pos("APP")] },
      ],
    };
    expect(computeAtcAnchors(atc, emptyBoundaries)).toEqual([
      expect.objectContaining({ type: "airport", icao: "KTWR" }),
    ]);
  });
});

/// VATUSA/OIS#477: `atc-centers` shipped the ZNY bowtie and executed in zero tests, and
/// `buildAtcHoverLayer` — the whole of #211's fix — was never imported by one.
describe("atc-centers (VATUSA/OIS#477)", () => {
  const boundaries: GeoJSON.FeatureCollection = {
    type: "FeatureCollection",
    features: [
      {
        type: "Feature",
        properties: { id: "ZNY" },
        geometry: { type: "Polygon", coordinates: [[[-74, 40], [-73, 40], [-73, 41], [-74, 40]]] },
      },
    ],
  } as GeoJSON.FeatureCollection;

  const centers = (layers: ReturnType<typeof buildAtcLayers>) =>
    layers.find((l) => l.id === "atc-centers");

  it("renders only the centers that are online", () => {
    const atc = { ...baseAtc(), centers: [{ id: "ZNY", positions: [] }] } as unknown as AtcData;
    const layer = centers(buildAtcLayers(atc, boundaries));
    expect(layer).toBeDefined();
    const data = layer!.props.data as GeoJSON.FeatureCollection;
    expect(data.features).toHaveLength(1);
  });

  it("renders no center layer at all when nobody is online", () => {
    // Not an empty layer — the layer is skipped, which is what keeps an offline map unshaded.
    expect(centers(buildAtcLayers(baseAtc(), boundaries))).toBeUndefined();
  });

  it("matches the online id case-insensitively", () => {
    const atc = { ...baseAtc(), centers: [{ id: "zny", positions: [] }] } as unknown as AtcData;
    expect(centers(buildAtcLayers(atc, boundaries))).toBeDefined();
  });
});

describe("buildAtcHoverLayer (VATUSA/OIS#477)", () => {
  const airport = (icao: string, kinds: number): AtcAnchor => ({
    type: "airport",
    lat: 41,
    lon: -87,
    icao,
    positions: Array.from({ length: kinds }, (_, i) => ({
      callsign: `${icao}_${i}`, frequency: "120.750",
      kind: (["DEL", "GND", "TWR", "ATIS"] as const)[i % 4], name: "", rating: 3, logon_time: "",
    })),
  }) as AtcAnchor;

  it("is pickable, invisible, and sized in pixels", () => {
    const layer = buildAtcHoverLayer([airport("KORD", 1)]);
    expect(layer.id).toBe("atc-hover");
    expect(layer.props.pickable).toBe(true);
    expect(layer.props.radiusUnits).toBe("pixels");
    expect(layer.props.radiusMinPixels).toBe(13);
    // Invisible but still hit-tested — the visible marker is a DOM element sitting underneath deck.
    expect(layer.props.getFillColor).toEqual([0, 0, 0, 0]);
  });

  it("grows the hotspot with the badge stack", () => {
    // A fixed radius left most of a multi-badge stack unpickable (#211), so the radius is per-anchor.
    const radius = buildAtcHoverLayer([]).props.getRadius as unknown as (a: AtcAnchor) => number;
    expect(radius(airport("KORD", 4))).toBeGreaterThan(radius(airport("KORD", 1)));
  });

  it("positions each circle at its anchor", () => {
    const pos = buildAtcHoverLayer([]).props.getPosition as unknown as (a: AtcAnchor) => number[];
    expect(pos(airport("KORD", 1))).toEqual([-87, 41]);
  });
});

/// VATUSA/OIS#481 — a validator nothing calls is worth nothing, so these pin the wiring rather than
/// the algorithm (which `geo.test.ts` covers against the real ZNY ring).
describe("ring topology is applied to both atc.ts paths (VATUSA/OIS#481)", () => {
  /// Two triangles sharing one vertex — a bridged ring, the shape that tessellates into a wedge
  /// spanning between the lobes.
  const bridged: number[][] = [
    [40, -80],
    [41, -80],
    [41, -79],
    [40, -80],
    [45, -75],
    [46, -75],
    [46, -74],
  ];

  it("splits a bridged TRACON ring instead of drawing a wedge across it", () => {
    const atc = { ...baseAtc(), tracons: [tracon({ rings: [bridged] })] };
    const polys = tracons(buildAtcLayers(atc, emptyBoundaries));
    expect(polys).toBeDefined();
    // One ring in, two lobes out — each drawn as its own polygon.
    expect((polys!.props.data as unknown[]).length).toBe(2);
  });

  it("splits a bridged ARTCC boundary on the centers path", () => {
    const boundaries: GeoJSON.FeatureCollection = {
      type: "FeatureCollection",
      features: [
        {
          type: "Feature",
          properties: { id: "ZZZ" },
          geometry: {
            type: "Polygon",
            coordinates: [[[-80, 40], [-80, 41], [-79, 41], [-80, 40], [-75, 45], [-75, 46], [-74, 46], [-80, 40]]],
          },
        } as GeoJSON.Feature,
      ],
    };
    const atc = { ...baseAtc(), centers: [{ id: "ZZZ", positions: [] }] } as unknown as AtcData;
    const layer = buildAtcLayers(atc, boundaries).find((l) => l.id === "atc-centers");
    expect(layer).toBeDefined();
    const data = layer!.props.data as GeoJSON.FeatureCollection;
    const g = data.features[0].geometry as GeoJSON.MultiPolygon;
    expect(g.type).toBe("MultiPolygon");
    expect(g.coordinates).toHaveLength(2);
  });
});
