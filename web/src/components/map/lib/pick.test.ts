import type {PickingInfo} from "@deck.gl/core";
import {describe, expect, it, vi} from "vitest";

import {fcaLineUnder, objectUnder} from "./pick";

/** A click on the invisible ATC hover target, with `under` as whatever `fca-lines` holds there. */
const atcPick = (under: unknown, pickObject = vi.fn().mockReturnValue(under)) =>
  ({
    x: 100,
    y: 200,
    layer: { id: "atc-hover", context: { deck: { pickObject } } },
  }) as unknown as PickingInfo;

describe("fcaLineUnder", () => {
  it("finds the FCA line hidden under an ATC pill", () => {
    const pickObject = vi.fn().mockReturnValue({ object: { id: "fca-7" } });
    expect(fcaLineUnder(atcPick(null, pickObject))).toBe("fca-7");
    // Only the FCA lines are re-picked — not the ATC target that was already on top.
    expect(pickObject).toHaveBeenCalledWith({ x: 100, y: 200, radius: 4, layerIds: ["fca-lines"] });
  });

  it("returns null when no line is under the pill", () => {
    expect(fcaLineUnder(atcPick(null))).toBeNull();
  });

  it("returns null when deck isn't reachable from the pick", () => {
    expect(fcaLineUnder({ x: 1, y: 2, layer: { id: "atc-hover" } } as unknown as PickingInfo)).toBeNull();
  });
});

describe("objectUnder", () => {
  it("re-picks only the layer asked for and hands back its object", () => {
    const pickObject = vi.fn().mockReturnValue({ object: { icao: "KBOI" } });
    expect(objectUnder(atcPick(null, pickObject), "atc-hover")).toEqual({ icao: "KBOI" });
    expect(pickObject).toHaveBeenCalledWith({ x: 100, y: 200, radius: 4, layerIds: ["atc-hover"] });
  });

  it("is null when that layer holds nothing there", () => {
    expect(objectUnder(atcPick(null), "atc-hover")).toBeNull();
  });
});
