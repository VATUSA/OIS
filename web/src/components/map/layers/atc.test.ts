import {describe, expect, it} from "vitest";

import {buildAtcLayers, computeAtcAnchors, type AtcData} from "./atc";

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
