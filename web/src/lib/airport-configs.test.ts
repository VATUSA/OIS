import {describe, expect, it} from "vitest";

import {inWindRange, matchConfig, type AirportConfig} from "./airport-configs";

/**
 * These mirror the Rust cases in `backend/src/repos/airport_configs.rs` one for one. `matchConfig` is a
 * hand-maintained port of `favored_config`, and until #510 this side had no tests at all — so a change
 * to one that missed the other was caught by nothing. Keep the two sets in step.
 */
const cfg = (over: Partial<AirportConfig>): AirportConfig => ({
  id: "c",
  icao: "KTST",
  name: "Config",
  aar: 30,
  adr: 30,
  landing_runways: [],
  wind_from_deg: 0,
  wind_to_deg: 360,
  calm_default: false,
  editable: true,
  artcc: "ZNY",
  updated_at: "2026-01-01T00:00:00Z",
  ...over,
});

const calm = cfg({ id: "calm", calm_default: true, wind_from_deg: 0, wind_to_deg: 0 });
const south = cfg({ id: "south", wind_from_deg: 150, wind_to_deg: 210 });
const west = cfg({ id: "west", wind_from_deg: 240, wind_to_deg: 300 });

describe("inWindRange", () => {
  it("handles a wrap-around range", () => {
    expect(inWindRange(355, 340, 20)).toBe(true);
    expect(inWindRange(10, 340, 20)).toBe(true);
    expect(inWindRange(180, 340, 20)).toBe(false);
  });

  it("treats both bounds as inclusive", () => {
    expect(inWindRange(150, 150, 210)).toBe(true);
    expect(inWindRange(210, 150, 210)).toBe(true);
  });
});

describe("matchConfig", () => {
  it("picks the config whose rule contains the wind", () => {
    expect(matchConfig([calm, south, west], 270)?.id).toBe("west");
    expect(matchConfig([calm, south, west], 180)?.id).toBe("south");
  });

  /** The #510 change: a gap takes the closest rule, not the calm default. */
  it("takes the closest rule when the wind falls in a gap", () => {
    const north = cfg({ id: "north", wind_from_deg: 340, wind_to_deg: 20 });
    // 180 is in neither; north's nearer edge (20) is 160 away, so it still wins over nothing.
    expect(matchConfig([calm, north], 180)?.id).toBe("north");
    // 220 is 10 from south's edge (210) and 20 from west's (240).
    expect(matchConfig([calm, south, west], 220)?.id).toBe("south");
  });

  /** The owner's tiebreak. */
  it("gives an equally close tie to the calm default", () => {
    // 225 is exactly 15 degrees outside both south (210) and west (240).
    expect(matchConfig([calm, south, west], 225)?.id).toBe("calm");
  });

  it("still returns a config when a tie has no calm default to defer to", () => {
    expect(matchConfig([south, west], 225)?.id).toBe("south");
  });

  it("prefers a containing rule over a merely nearer edge", () => {
    const northwest = cfg({ id: "northwest", wind_from_deg: 305, wind_to_deg: 345 });
    // northwest's edge is 5 away, but west actually contains 300.
    expect(matchConfig([west, northwest], 300)?.id).toBe("west");
  });

  it("takes the calm default for calm or unknown wind", () => {
    expect(matchConfig([calm, south, west], null)?.id).toBe("calm");
    expect(matchConfig([calm, south, west], undefined)?.id).toBe("calm");
  });

  it("falls back to the first config when there is no calm default", () => {
    expect(matchConfig([south, west], null)?.id).toBe("south");
  });

  it("returns undefined when there are no configs at all", () => {
    expect(matchConfig([], 270)).toBeUndefined();
  });
});
