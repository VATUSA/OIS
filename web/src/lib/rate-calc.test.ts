import {describe, expect, it} from "vitest";

import {type Positions, recommendedAar, tierForIcao,} from "./rate-calc";

const allOff: Positions = {
  app: false,
  twr: false,
  gnd: false,
  del: false,
  sup: false,
  atis: false,
};

describe("recommendedAar", () => {
  it("scales max AAR by the staffing ratio", () => {
    // vatflow's worked example: a hub at 120/hr, 6 full, 2 on duty -> 40/hr.
    const r = recommendedAar({
      maxAar: 120,
      fullStaff: 6,
      onDuty: 2,
      usePositions: false,
      positions: allOff,
    });
    expect(r.aar).toBe(40);
    expect(r.pct).toBe(33);
    expect(r.limitedBy).toBe("headcount");
  });

  it("returns full capacity when fully staffed", () => {
    const r = recommendedAar({
      maxAar: 80,
      fullStaff: 5,
      onDuty: 5,
      usePositions: false,
      positions: allOff,
    });
    expect(r.aar).toBe(80);
    expect(r.pct).toBe(100);
  });

  it("warns and returns 0 with nobody on duty", () => {
    const r = recommendedAar({
      maxAar: 45,
      fullStaff: 4,
      onDuty: 0,
      usePositions: false,
      positions: allOff,
    });
    expect(r.aar).toBe(0);
    expect(r.limitedBy).toBe("staff");
    expect(r.warning).toMatch(/on duty/i);
  });

  it("applies the critical cap when neither APP nor TWR is open", () => {
    // GND+DEL+SUP+ATIS = 0.25 position weight, but with no APP/TWR the critical
    // cap (0.15) takes over.
    const r = recommendedAar({
      maxAar: 100,
      fullStaff: 3,
      onDuty: 3,
      usePositions: true,
      positions: { ...allOff, gnd: true, del: true, sup: true, atis: true },
    });
    expect(r.limitedBy).toBe("critical");
    expect(r.aar).toBe(15); // 0.15 * 100
  });

  it("is position-limited (not critical) when the mix stays under the cap", () => {
    // Only GND (0.12) is below the 0.15 cap, so it's limited by the position mix.
    const r = recommendedAar({
      maxAar: 100,
      fullStaff: 3,
      onDuty: 3,
      usePositions: true,
      positions: { ...allOff, gnd: true },
    });
    expect(r.limitedBy).toBe("positions");
    expect(r.aar).toBe(12);
  });

  it("uses the position weight cap when APP+TWR are open", () => {
    // APP (0.40) + TWR (0.35) = 0.75 cap, below the headcount ratio of 1.
    const r = recommendedAar({
      maxAar: 100,
      fullStaff: 2,
      onDuty: 2,
      usePositions: true,
      positions: { ...allOff, app: true, twr: true },
    });
    expect(r.limitedBy).toBe("positions");
    expect(r.aar).toBe(75);
  });
});

describe("tierForIcao", () => {
  it("classifies known airports and defaults to small", () => {
    expect(tierForIcao("KATL")).toBe("hub");
    expect(tierForIcao("KBOS")).toBe("large");
    expect(tierForIcao("KDCA")).toBe("medium");
    expect(tierForIcao("KABC")).toBe("small");
  });
});
