import {describe, expect, it} from "vitest";

import {movedPath} from "./moved-paths";

describe("movedPath", () => {
  it("moves planning and historical subtrees under /admin, keeping params and search", () => {
    expect(movedPath("/planning", "")).toBe("/admin/planning");
    expect(movedPath("/planning/events/42/fcas", "")).toBe("/admin/planning/events/42/fcas");
    expect(movedPath("/historical/flights/abc", "")).toBe("/admin/historical/flights/abc");
    expect(movedPath("/historical/replay", "?capture=x&from=1")).toBe("/admin/historical/replay?capture=x&from=1");
  });

  it("leaves other paths alone", () => {
    expect(movedPath("/planningx", "")).toBeNull();
    expect(movedPath("/admin/planning/events", "")).toBeNull();
    expect(movedPath("/ops/airport", "")).toBeNull();
  });
});
