import {describe, expect, it} from "vitest";

import type {Me} from "./auth";
import {hasPermission, isAdmin} from "./permissions";

function me(overrides: Partial<Me>): Me {
  return {
    id: "u1",
    cid: 1,
    email: "a@b.c",
    display_name: "Tester",
    rating: null,
    server_admin: false,
    role_names: [],
    permissions: {},
    ...overrides,
  } as Me;
}

describe("hasPermission", () => {
  it("returns false without a user", () => {
    expect(hasPermission(null, "tmu.program.read")).toBe(false);
    expect(hasPermission(undefined, "tmu.program.read")).toBe(false);
  });

  it("grants everything to a server admin", () => {
    expect(hasPermission(me({ server_admin: true }), "anything.at.all")).toBe(true);
  });

  it("reads a nested segments.action permission tree", () => {
    const u = me({ permissions: { tmu: { program: ["read", "update"] } } });
    expect(hasPermission(u, "tmu.program.read")).toBe(true);
    expect(hasPermission(u, "tmu.program.update")).toBe(true);
    expect(hasPermission(u, "tmu.program.delete")).toBe(false);
    expect(hasPermission(u, "tmu.tmi.read")).toBe(false);
  });

  it("returns false when the path is missing", () => {
    const u = me({ permissions: { tmu: { program: ["read"] } } });
    expect(hasPermission(u, "audit.logs.read")).toBe(false);
  });
});

describe("isAdmin", () => {
  it("is true for a server admin", () => {
    expect(isAdmin(me({ server_admin: true }))).toBe(true);
  });
  it("is true when holding an admin permission", () => {
    expect(isAdmin(me({ permissions: { audit: { logs: ["read"] } } }))).toBe(true);
  });
  it("is false otherwise", () => {
    expect(isAdmin(me({ permissions: { tmu: { program: ["read"] } } }))).toBe(false);
    expect(isAdmin(null)).toBe(false);
  });
});
