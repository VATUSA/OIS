import {describe, expect, it} from "vitest";

import type {Me} from "./auth";
import {inRestrictionScope, restrictionFacilities} from "./restriction-scope";

function me(overrides: Partial<Me>): Me {
  return {
    id: "u1",
    cid: 1,
    email: "a@b.c",
    display_name: "Tester",
    rating: null,
    server_admin: false,
    role_names: [],
    tmu_national: false,
    permissions: {} as Me["permissions"],
    ...overrides,
  } as Me;
}

const vatusa = (home: string | null, visits: string[] = []) =>
  ({ home_facility: home, visits }) as NonNullable<Me["vatusa"]>;

describe("restrictionFacilities", () => {
  it("is the home facility plus the ones they visit", () => {
    const f = restrictionFacilities(me({ vatusa: vatusa("ZDC", ["ZNY", "ZOB"]) }));
    expect(f).not.toBeNull();
    expect([...f!].sort()).toEqual(["ZDC", "ZNY", "ZOB"]);
  });

  it("is every ARTCC for a national TMU reader", () => {
    expect(restrictionFacilities(me({ tmu_national: true, vatusa: vatusa("ZDC") }))).toBeNull();
  });

  it("is nothing — not everything — before the VATUSA profile has synced", () => {
    expect(restrictionFacilities(me({}))?.size).toBe(0);
    expect(restrictionFacilities(null)?.size).toBe(0);
  });

  it("does not hand a plain server_admin the whole country through this path", () => {
    // `tmu_national` is the national test; it already accounts for SERVER_ADMIN server-side, so a
    // `server_admin` flag alone must not widen the audience here.
    expect(restrictionFacilities(me({ server_admin: true, vatusa: vatusa("ZDC") }))).not.toBeNull();
  });
});

describe("inRestrictionScope", () => {
  const zdc = new Set(["ZDC"]);

  it("matches a restriction at one of the user's ARTCCs", () => {
    expect(inRestrictionScope(zdc, ["ZDC"])).toBe(true);
    expect(inRestrictionScope(zdc, ["ZLA"])).toBe(false);
  });

  it("matches a TMI on either side", () => {
    expect(inRestrictionScope(zdc, ["ZDC", "ZLA"])).toBe(true);
    expect(inRestrictionScope(zdc, ["ZLA", "ZDC"])).toBe(true);
    expect(inRestrictionScope(zdc, ["ZLA", "ZNY"])).toBe(false);
  });

  // Fail open: a restriction the map couldn't place (a 3-letter id, a lowercase event TMI) used to
  // reach no one outside national TMU (VATUSA/OIS#405 review).
  it("lets a restriction through when none of its ARTCCs could be resolved", () => {
    expect(inRestrictionScope(zdc, [null])).toBe(true);
    expect(inRestrictionScope(zdc, [undefined, null])).toBe(true);
  });

  it("scopes on the side that did resolve when only one did", () => {
    expect(inRestrictionScope(zdc, [null, "ZDC"])).toBe(true);
    expect(inRestrictionScope(zdc, [null, "ZLA"])).toBe(false);
  });

  it("lets a national reader through, including an unresolved ARTCC", () => {
    expect(inRestrictionScope(null, ["ZLA"])).toBe(true);
    expect(inRestrictionScope(null, [null])).toBe(true);
  });
});
