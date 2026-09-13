import {describe, expect, it} from "vitest";

import {centroid, haversine, lineNm, midpointOf, normalizeLng, toDeckPath, toDeckPoint} from "./geo";

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
