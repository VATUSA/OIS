// @vitest-environment jsdom
import {afterEach, describe, expect, it, vi} from "vitest";

import {can, capabilities, invokeDesktop, isTauri, platform, type Capability} from "./platform";

// The real module talks to Tauri's IPC, which doesn't exist under test. Mocking it also proves
// invokeDesktop reaches @tauri-apps/api through its dynamic import rather than a static one.
const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({invoke: (...args: unknown[]) => invoke(...args)}));

/** Stand in for the global the Tauri v2 runtime injects into its webview. */
function pretendDesktop() {
  window.__TAURI_INTERNALS__ = {};
}

afterEach(() => {
  delete window.__TAURI_INTERNALS__;
  invoke.mockReset();
});

describe("platform detection", () => {
  it("reports web when Tauri's global is absent", () => {
    expect(isTauri()).toBe(false);
    expect(platform()).toBe("web");
  });

  it("reports desktop once Tauri's global is present", () => {
    pretendDesktop();
    expect(isTauri()).toBe(true);
    expect(platform()).toBe("desktop");
  });
});

describe("capabilities", () => {
  it("reports every capability absent on the web build", () => {
    const caps = capabilities();
    expect(Object.values(caps)).not.toHaveLength(0);
    for (const [name, available] of Object.entries(caps)) {
      expect(available, `${name} must be false on web`).toBe(false);
      expect(can(name as Capability)).toBe(false);
    }
  });

  it("still reports a capability absent on desktop until its feature ships", () => {
    // Every entry is false today; each is flipped on by the issue that implements it (#348-#354).
    // A consumer written now against can("tray") is therefore correct both before and after.
    pretendDesktop();
    for (const [name, available] of Object.entries(capabilities())) {
      expect(available, `${name} is not implemented yet`).toBe(false);
    }
  });

  it("hands back a frozen snapshot a caller can't corrupt for everyone else", () => {
    const caps = capabilities();
    expect(Object.isFrozen(caps)).toBe(true);
    expect(capabilities().notifications).toBe(false);
  });
});

describe("invokeDesktop", () => {
  it("refuses on the web build instead of silently doing nothing", async () => {
    await expect(invokeDesktop("get_token")).rejects.toThrow(/web build/);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("names the offending command so the ungated caller is findable", async () => {
    await expect(invokeDesktop("get_token")).rejects.toThrow(/get_token/);
  });

  it("forwards command and args to Tauri on desktop", async () => {
    pretendDesktop();
    invoke.mockResolvedValue("ok");
    await expect(invokeDesktop<string>("get_token", {cid: 1234})).resolves.toBe("ok");
    expect(invoke).toHaveBeenCalledWith("get_token", {cid: 1234});
  });
});
