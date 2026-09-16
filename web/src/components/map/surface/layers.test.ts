import {describe, expect, it} from "vitest";

import type {
  AirportGate,
  AirportRampArea,
  AirportRunway,
  AirportSurface,
  AirportTaxiway,
} from "@/lib/airport-surface";

import {
  MIN_SURFACE_POINTS,
  buildSurfaceDraftLayers,
  buildSurfaceLayers,
  isPolygonKind,
} from "./layers";

const gate = (over: Partial<AirportGate>): AirportGate => ({
  id: "g1",
  icao: "KTST",
  name: "A1",
  lat: 38.85,
  lon: -77.04,
  source: "manual",
  editable: true,
  updated_at: "2026-01-01T00:00:00Z",
  ...over,
});

const rampArea = (over: Partial<AirportRampArea>): AirportRampArea => ({
  id: "r1",
  icao: "KTST",
  name: "North apron",
  kind: "apron",
  rings: [
    [
      [38.85, -77.04],
      [38.86, -77.04],
      [38.86, -77.05],
      [38.85, -77.04],
    ],
  ],
  source: "manual",
  editable: true,
  updated_at: "2026-01-01T00:00:00Z",
  ...over,
});

const taxiway = (over: Partial<AirportTaxiway>): AirportTaxiway => ({
  id: "t1",
  icao: "KTST",
  name: "A",
  rings: [
    [
      [38.85, -77.04],
      [38.86, -77.05],
      [38.85, -77.05],
      [38.85, -77.04],
    ],
  ],
  source: "manual",
  editable: true,
  updated_at: "2026-01-01T00:00:00Z",
  ...over,
});

const runway = (over: Partial<AirportRunway>): AirportRunway => ({
  id: "rw1",
  icao: "KTST",
  name: "01/19",
  rings: [
    [
      [38.85, -77.04],
      [38.87, -77.04],
      [38.87, -77.041],
      [38.85, -77.04],
    ],
  ],
  source: "faa",
  editable: true,
  updated_at: "2026-01-01T00:00:00Z",
  ...over,
});

const surface = (over: Partial<AirportSurface>): AirportSurface => ({
  gates: [],
  ramp_areas: [],
  taxiways: [],
  runways: [],
  ...over,
});

const dataIds = (layers: ReturnType<typeof buildSurfaceLayers>, id: string) => {
  const layer = layers.find((l) => l.id === id);
  return ((layer?.props.data ?? []) as { id: string }[]).map((d) => d.id);
};

describe("buildSurfaceLayers", () => {
  it("includes every gate/ramp area/taxiway when nothing is selected", () => {
    const s = surface({
      gates: [gate({ id: "a" }), gate({ id: "b" })],
      ramp_areas: [rampArea({ id: "r" })],
      taxiways: [taxiway({ id: "t" })],
    });
    const layers = buildSurfaceLayers(s, null);
    expect(dataIds(layers, "surface-gates")).toEqual(["a", "b"]);
    expect(dataIds(layers, "surface-ramp-areas")).toEqual(["r"]);
    expect(dataIds(layers, "surface-taxiways")).toEqual(["t"]);
  });

  it("excludes the selected gate — it's rendered by the draft layers instead, not twice", () => {
    const s = surface({ gates: [gate({ id: "a" }), gate({ id: "b" })] });
    const layers = buildSurfaceLayers(s, { kind: "gate", id: "a" });
    expect(dataIds(layers, "surface-gates")).toEqual(["b"]);
  });

  it("excludes the selected ramp area without affecting gates or taxiways of the same id", () => {
    // Same id string reused across kinds — selection must be scoped by kind, not just id.
    const s = surface({
      gates: [gate({ id: "shared" })],
      ramp_areas: [rampArea({ id: "shared" }), rampArea({ id: "other" })],
    });
    const layers = buildSurfaceLayers(s, { kind: "ramp", id: "shared" });
    expect(dataIds(layers, "surface-ramp-areas")).toEqual(["other"]);
    expect(dataIds(layers, "surface-gates")).toEqual(["shared"]);
  });

  it("excludes the selected taxiway", () => {
    const s = surface({ taxiways: [taxiway({ id: "a" }), taxiway({ id: "b" })] });
    const layers = buildSurfaceLayers(s, { kind: "taxiway", id: "b" });
    expect(dataIds(layers, "surface-taxiways")).toEqual(["a"]);
  });

  it("omits a layer entirely once its list is empty (nothing left after excluding the sole item)", () => {
    const s = surface({ gates: [gate({ id: "a" })] });
    const layers = buildSurfaceLayers(s, { kind: "gate", id: "a" });
    expect(layers.find((l) => l.id === "surface-gates")).toBeUndefined();
  });
});

describe("buildSurfaceDraftLayers", () => {
  it("renders only vertices for a single-point gate draft", () => {
    const layers = buildSurfaceDraftLayers("gate", [[38.85, -77.04]], "edit");
    expect(layers.map((l) => l.id)).toEqual(["surface-draft-vertices"]);
  });

  it("renders an open dashed line for a taxiway/ramp draft still being drawn", () => {
    const layers = buildSurfaceDraftLayers(
      "ramp",
      [
        [38.85, -77.04],
        [38.86, -77.04],
      ],
      "draw",
    );
    expect(layers.map((l) => l.id)).toContain("surface-draft-line");
    expect(layers.map((l) => l.id)).not.toContain("surface-draft-polygon");
  });

  it.each(["ramp", "taxiway"] as const)(
    "closes a %s into a filled polygon only in edit phase with >= 3 points (#278)",
    (kind) => {
      const points: [number, number][] = [
        [38.85, -77.04],
        [38.86, -77.04],
        [38.86, -77.05],
      ];
      const drawing = buildSurfaceDraftLayers(kind, points, "draw");
      expect(drawing.map((l) => l.id)).not.toContain("surface-draft-polygon");

      const edited = buildSurfaceDraftLayers(kind, points, "edit");
      expect(edited.map((l) => l.id)).toContain("surface-draft-polygon");
    },
  );

  it("renders runways as a filled polygon layer and leaves out the selected one (#279)", () => {
    const s = surface({ runways: [runway({ id: "a" }), runway({ id: "b" })] });
    const all = buildSurfaceLayers(s, null).find((l) => l.id === "surface-runways");
    expect(all?.constructor.name).toBe("PolygonLayer");
    expect(dataIds(buildSurfaceLayers(s, { kind: "runway", id: "a" }), "surface-runways")).toEqual(["b"]);
  });

  it("renders saved taxiways as a filled polygon layer, like ramp areas (#278)", () => {
    const layers = buildSurfaceLayers(surface({ taxiways: [taxiway({ id: "t" })] }), null);
    const layer = layers.find((l) => l.id === "surface-taxiways");
    expect(layer?.constructor.name).toBe("PolygonLayer");
    expect(layer?.props).toMatchObject({ filled: true });
  });
});

describe("runway polygons (#279)", () => {
  it("a runway is a polygon kind needing 3 points, like taxiways and ramps", () => {
    expect(isPolygonKind("runway")).toBe(true);
    expect(MIN_SURFACE_POINTS.runway).toBe(MIN_SURFACE_POINTS.taxiway);
  });

  it("draws larger pavement first so the smaller shape on top stays pickable", () => {
    const layers = buildSurfaceLayers(
      surface({
        ramp_areas: [rampArea({ id: "r" })],
        runways: [runway({ id: "rw" })],
        taxiways: [taxiway({ id: "t" })],
        gates: [gate({ id: "g" })],
      }),
      null,
    );
    expect(layers.map((l) => l.id)).toEqual([
      "surface-ramp-areas",
      "surface-runways",
      "surface-taxiways",
      "surface-gates",
    ]);
  });
});

describe("polygon geometry (#278)", () => {
  it("hands deck.gl [lon, lat] rings, not the API's [lat, lon]", () => {
    const s = surface({
      taxiways: [taxiway({ id: "t" })],
      ramp_areas: [rampArea({ id: "r" })],
    });
    const layers = buildSurfaceLayers(s, null);
    for (const [id, item] of [
      ["surface-taxiways", taxiway({ id: "t" })],
      ["surface-ramp-areas", rampArea({ id: "r" })],
    ] as const) {
      const layer = layers.find((l) => l.id === id);
      const { getPolygon } = layer?.props as unknown as {
        getPolygon: (d: typeof item) => number[][][];
      };
      expect(getPolygon(item), id).toEqual(item.rings.map((ring) => ring.map(([lat, lon]) => [lon, lat])));
    }
  });

  it("a taxiway needs 3 points before it can be finalized, like a ramp", () => {
    expect(MIN_SURFACE_POINTS.taxiway).toBe(3);
    expect(MIN_SURFACE_POINTS.taxiway).toBe(MIN_SURFACE_POINTS.ramp);
  });

  // Superseded by #279's largest-first order: a taxiway crossing an apron is the smaller shape, so
  // it sits on top and wins the click; the apron stays pickable everywhere else.
  it("draws taxiways above ramp areas so pavement crossing an apron stays pickable", () => {
    const layers = buildSurfaceLayers(
      surface({ taxiways: [taxiway({ id: "t" })], ramp_areas: [rampArea({ id: "r" })] }),
      null,
    );
    const ids = layers.map((l) => l.id);
    expect(ids.indexOf("surface-ramp-areas")).toBeLessThan(ids.indexOf("surface-taxiways"));
  });
});
