import {describe, expect, it} from "vitest";

import {planAt, type Plan} from "./replay";

const KBOS: Plan = { t: 0, actype: "B738", dep: "KJFK", arr: "KBOS", route: "R1" };
const KPHL: Plan = { t: 600, actype: "B738", dep: "KJFK", arr: "KPHL", route: "R2" };

describe("planAt", () => {
  it("returns the plan in effect at the clock (last t <= clock)", () => {
    const plans = [KBOS, KPHL];
    expect(planAt(plans, 0).arr).toBe("KBOS"); // window open
    expect(planAt(plans, 599).arr).toBe("KBOS"); // just before the amendment
    expect(planAt(plans, 600).arr).toBe("KPHL"); // at the amendment instant
    expect(planAt(plans, 5000).arr).toBe("KPHL"); // after
  });

  it("clamps below the first revision to the first plan", () => {
    // A flight that connected mid-window: its first revision has t > 0.
    expect(planAt([KPHL], 0).arr).toBe("KPHL");
  });

  it("handles a single (never-amended) plan and an empty list", () => {
    expect(planAt([KBOS], 9999).arr).toBe("KBOS");
    expect(planAt([], 100).arr).toBe(""); // blank fallback
  });
});
