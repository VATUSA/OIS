import {describe, expect, it} from "vitest";

import type {AirportGate, AirportRampArea, AirportSurface, AirportTaxiway} from "@/lib/airport-surface";

import {buildSurfaceDraftLayers, buildSurfaceLayers} from "./layers";

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
  points: [
    [38.85, -77.04],
    [38.86, -77.05],
  ],
  source: "manual",
  editable: true,
  updated_at: "2026-01-01T00:00:00Z",
  ...over,
});

const surface = (over: Partial<AirportSurface>): AirportSurface => ({
  gates: [],
  ramp_areas: [],
  taxiways: [],
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

  it("closes into a filled polygon only for a ramp in edit phase with >= 3 points", () => {
    const points: [number, number][] = [
      [38.85, -77.04],
      [38.86, -77.04],
      [38.86, -77.05],
    ];
    const drawing = buildSurfaceDraftLayers("ramp", points, "draw");
    expect(drawing.map((l) => l.id)).not.toContain("surface-draft-polygon");

    const edited = buildSurfaceDraftLayers("ramp", points, "edit");
    expect(edited.map((l) => l.id)).toContain("surface-draft-polygon");
  });

  it("never closes a taxiway into a polygon, regardless of phase or point count", () => {
    const points: [number, number][] = [
      [38.85, -77.04],
      [38.86, -77.04],
      [38.86, -77.05],
    ];
    const layers = buildSurfaceDraftLayers("taxiway", points, "edit");
    expect(layers.map((l) => l.id)).not.toContain("surface-draft-polygon");
    expect(layers.map((l) => l.id)).toContain("surface-draft-line");
  });
});
