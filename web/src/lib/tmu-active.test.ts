import {describe, expect, it} from "vitest";

import {isActiveTmi, type Tmi} from "./tmu";

const tmi = (status: string) => ({status}) as unknown as Tmi;

describe("isActiveTmi", () => {
  // The dashboard and the menu-bar tray both count with this; the tray used to count every TMI the
  // list returned, drafts and cancelled included (VATUSA/OIS#351 review).
  it("counts a published TMI and nothing else", () => {
    expect(isActiveTmi(tmi("published"))).toBe(true);
    for (const status of ["draft", "expired", "cancelled"]) expect(isActiveTmi(tmi(status))).toBe(false);
  });
});
