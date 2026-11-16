import {describe, expect, it} from "vitest";

import {angleDiff, presetEnds, type RunwayEnd, starBase} from "./runway";

/**
 * The Runway Balancer's preset and STAR-key rules (#956). Every expected value is a literal worked
 * out by hand, never derived from `PRESET_TOLERANCE_DEG` or `PRESET_HEADING`, so a changed tolerance,
 * heading or regex turns a case red.
 */

function end(id: string, hdg: number, active = false): RunwayEnd {
  return { id, hdg, len: 10000, active, pair: `${id}/x` };
}

/** KJFK-style ends: each cardinal selects a different four. */
const KJFK = [
  end("04L", 44),
  end("04R", 44),
  end("13L", 134),
  end("13R", 134),
  end("22L", 224),
  end("22R", 224),
  end("31L", 314),
  end("31R", 314),
];

describe("presetEnds", () => {
  it("for W keeps ends at 205 and 335 and drops ends at 204 and 336", () => {
    const ends = [end("a", 204), end("b", 205), end("c", 335), end("d", 336)];
    expect(presetEnds(ends, "W")).toEqual(["b", "c"]);
  });

  it("for N keeps ends at 295 and 65 and drops ends at 294 and 66", () => {
    const ends = [end("a", 294), end("b", 295), end("c", 65), end("d", 66)];
    expect(presetEnds(ends, "N")).toEqual(["b", "c"]);
  });

  it("for N keeps ends at 0, 5 and 360", () => {
    const ends = [end("a", 0), end("b", 5), end("c", 360)];
    expect(presetEnds(ends, "N")).toEqual(["a", "b", "c"]);
  });

  it("selects each cardinal's ends from a KJFK end set", () => {
    expect(presetEnds(KJFK, "W")).toEqual(["22L", "22R", "31L", "31R"]);
    expect(presetEnds(KJFK, "E")).toEqual(["04L", "04R", "13L", "13R"]);
    expect(presetEnds(KJFK, "N")).toEqual(["04L", "04R", "31L", "31R"]);
    expect(presetEnds(KJFK, "S")).toEqual(["13L", "13R", "22L", "22R"]);
  });

  it("for OFF returns [] even when ends are active and face a cardinal", () => {
    const ends = [end("22R", 224, true), end("31L", 314, true)];
    expect(presetEnds(ends, "W")).toEqual(["22R", "31L"]);
    expect(presetEnds(ends, "OFF")).toEqual([]);
  });

  it("returns [] for an unknown preset", () => {
    for (const preset of ["X", "w", ""]) {
      expect(presetEnds(KJFK, preset), JSON.stringify(preset)).toEqual([]);
    }
  });

  it("returns [] for an empty end list", () => {
    for (const preset of ["W", "E", "N", "S", "OFF"]) {
      expect(presetEnds([], preset), preset).toEqual([]);
    }
  });
});

describe("angleDiff", () => {
  it("measures the short way round, in either order", () => {
    expect(angleDiff(350, 10)).toBe(20);
    expect(angleDiff(10, 350)).toBe(20);
  });

  it("measures 0 vs 360 as 0 and 90 vs 270 as 180", () => {
    expect(angleDiff(0, 360)).toBe(0);
    expect(angleDiff(90, 270)).toBe(180);
  });
});

describe("starBase", () => {
  // The same literals as the backend's `star_base_strips_revision` (backend/src/feed/runway.rs):
  // the backend matches saved rule keys against `star_base` exactly, so the two must agree.
  it("matches the backend star_base on its literal cases", () => {
    expect(starBase("CAMRN4")).toBe("CAMRN");
    expect(starBase("DOTSS2A")).toBe("DOTSS");
    expect(starBase("lendy6")).toBe("LENDY");
    expect(starBase("PARCH")).toBe("PARCH");
    expect(starBase("BAMBI3")).toBe("BAMBI");
    expect(starBase("ANJLL4A")).toBe("ANJLL");
  });

  it("trims surrounding whitespace before stripping", () => {
    expect(starBase("  camrn4  ")).toBe("CAMRN");
  });

  it("strips only the last revision", () => {
    expect(starBase("CAMRN44A")).toBe("CAMRN4");
    expect(starBase("CAMRN44")).toBe("CAMRN4");
  });

  it("keeps a trailing letter with no digit before it", () => {
    expect(starBase("CAMRNA")).toBe("CAMRNA");
  });
});
