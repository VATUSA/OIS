// @vitest-environment jsdom
import {afterEach, describe, expect, it, vi} from "vitest";

// One capability implemented — the state #348 creates when it flips `notifications`. With every flag
// `false`, `can()` answers `false` on both platforms whether or not it consults the desktop gate, so
// only this can see a `can()` (or `capabilities()`) that has stopped checking (VATUSA/OIS#345).
vi.mock("./platform-flags", () => ({
  IMPLEMENTED: Object.freeze({
    autoUpdate: false,
    notifications: true,
    miniWindows: false,
    multiWindow: false,
    tray: false,
    globalHotkeys: false,
    audioAlerts: false,
    fileDialogs: false,
  }),
}));

import {can, capabilities} from "./platform";

afterEach(() => {
  delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
});

describe("the desktop gate, with a capability implemented (VATUSA/OIS#345)", () => {
  it("withholds it from the web build", () => {
    expect(can("notifications")).toBe(false);
    expect(capabilities().notifications).toBe(false);
  });

  it("offers it inside the desktop shell", () => {
    (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
    expect(can("notifications")).toBe(true);
    expect(capabilities().notifications).toBe(true);
  });
});
