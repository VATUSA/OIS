import {describe, expect, it} from "vitest";

import type {Me} from "./auth";
import {hasPermission} from "./permissions";

function me(overrides: { server_admin?: boolean; permissions?: unknown }): Me {
  return {
    id: "u1",
    cid: 1,
    email: "a@b.c",
    display_name: "Tester",
    rating: null,
    server_admin: overrides.server_admin ?? false,
    role_names: [],
    permissions: (overrides.permissions ?? {}) as Me["permissions"],
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
