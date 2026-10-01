// @vitest-environment jsdom
import {afterEach, describe, expect, it, vi} from "vitest";

import {
  availability,
  can,
  capabilities,
  invokeDesktop,
  isMacOS,
  isMainWindow,
  isTauri,
  platform,
  windowLabel,
  type Capability,
} from "./platform";

// The real module talks to Tauri's IPC, which doesn't exist under test, so we stand in for it.
// Note this mock says NOTHING about static-vs-dynamic importing — vi.mock intercepts both forms
// identically, and this suite stays green if platform.ts is converted to a top-level import.
// Bundle isolation is enforced solely by the `no-restricted-imports` rule in eslint.config.mjs.
const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({invoke: (...args: unknown[]) => invoke(...args)}));

const currentLabel = vi.fn(() => "main");
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({get label() {return currentLabel();}}),
}));

/** Stand in for the global the Tauri v2 runtime injects into its webview. */
function pretendDesktop() {
  window.__TAURI_INTERNALS__ = {};
}

/** The real user agents of the three webviews Tauri runs the app in. */
const AGENTS = {
  wkWebView:
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15",
  webView2:
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
  webKitGtk:
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15",
} as const;

function pretendAgent(ua: string) {
  Object.defineProperty(navigator, "userAgent", {value: ua, configurable: true});
}

afterEach(() => {
  delete window.__TAURI_INTERNALS__;
  invoke.mockReset();
  pretendAgent(AGENTS.webView2);
  // Back to the default label: tests below install their own, including one that throws, and a
  // leaked implementation would decide a later test's answer.
  currentLabel.mockImplementation(() => "main");
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

  it("reports a capability absent on desktop until its own feature ships", () => {
    // A capability turns on only when the issue that implements it flips its flag — so a consumer
    // written against can("tray") today is correct both before and after #351 lands. Updating this
    // list is the deliberate cost of that: flipping a flag without noticing fails here.
    pretendDesktop();
    const shipped: Capability[] = [
      "autoUpdate",
      "notifications",
      "miniWindows",
      "multiWindow",
      "tray",
      "globalHotkeys",
      "audioAlerts",
      "windowControls",
    ]; // #347-#353, #402
    for (const [name, available] of Object.entries(capabilities())) {
      const expected = shipped.includes(name as Capability);
      expect(available, `${name} availability on desktop`).toBe(expected);
    }
  });

  it("reports even a shipped capability absent on the web build", () => {
    // Both are implemented, but a website has nothing to update and no OS notification centre —
    // the platform gate still has to hold or web callers would take a desktop-only path.
    expect(can("autoUpdate")).toBe(false);
    expect(can("notifications")).toBe(false);
  });

  // The gate this module exists for. It has to be asserted on `availability` rather than through
  // `can()`, because every entry in IMPLEMENTED is false today — so `can()` returns false on both
  // platforms whether or not it consults isTauri(), and an inlined guard could be deleted outright
  // without a single test noticing. This is what arms the leak when a feature issue flips a flag.
  it("withholds an implemented capability from the web build", () => {
    expect(availability(false, true)).toBe(false);
    expect(availability(true, true)).toBe(true);
    expect(availability(true, false)).toBe(false);
    expect(availability(false, false)).toBe(false);
  });

  it("keeps one snapshot identity per platform so it is safe in a dependency array", () => {
    expect(capabilities()).toBe(capabilities());
    pretendDesktop();
    expect(capabilities()).toBe(capabilities());
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

describe("window identity", () => {
  it("has no window label on the web build", async () => {
    await expect(windowLabel()).resolves.toBeUndefined();
    await expect(isMainWindow()).resolves.toBe(false);
  });

  it("recognises the primary window", async () => {
    pretendDesktop();
    currentLabel.mockReturnValue("main");

    await expect(windowLabel()).resolves.toBe("main");
    await expect(isMainWindow()).resolves.toBe(true);
  });

  /**
   * The guard that stops a route window or a pop-out rotating the session token out from under the
   * main window. Every Tauri webview runs the same entry module, so "am I the main window?" is the
   * only thing separating launch-once work from work that runs per window.
   */
  it("does not mistake a route window for the primary window", async () => {
    pretendDesktop();
    currentLabel.mockReturnValue("window--ops-idst");
    await expect(isMainWindow()).resolves.toBe(false);
  });

  it("does not mistake a pop-out for the primary window", async () => {
    pretendDesktop();
    currentLabel.mockReturnValue("popout-fca-ZDC");

    await expect(windowLabel()).resolves.toBe("popout-fca-ZDC");
    await expect(isMainWindow()).resolves.toBe(false);
  });

  /**
   * #403: "couldn't tell" must answer *not* the main window, and this is the only place that
   * answer is made. `window-controls.tsx` and `popout.ts` each had their own `catch` until the
   * three copies were collapsed into this helper; now the fallback below is the only thing
   * standing between an unreadable window and a route window that thinks it is `main` — drawing a
   * second set of controls over the OS's, and, in `restoreWindows`, relaunching the whole set from
   * a window that was itself restored.
   *
   * Flipping the fallback to `MAIN_WINDOW_LABEL`, or deleting the `catch` so the throw escapes,
   * must fail here.
   */
  it("is not the primary window when the window cannot be read", async () => {
    pretendDesktop();
    currentLabel.mockImplementation(() => {
      throw new Error("window unavailable");
    });

    await expect(windowLabel()).resolves.toBeUndefined();
    await expect(isMainWindow()).resolves.toBe(false);
  });
});


/**
 * `isMacOS` decides who draws the window's buttons (#419), and it is the one predicate here with no
 * Tauri call behind it to stand in for — it reads the user agent directly. Both wrong answers are
 * shipping defects, which is why both directions are pinned: answering `true` on Windows draws no
 * replica in an undecorated window, leaving it with no close button at all; answering `false` on
 * macOS draws a replica on top of the real traffic lights the OS already put there.
 */
describe("host OS", () => {
  it("knows macOS from WKWebView's user agent", () => {
    pretendDesktop();
    pretendAgent(AGENTS.wkWebView);
    expect(isMacOS()).toBe(true);
  });

  it("does not mistake Windows' WebView2 for macOS, or the window loses its only close button", () => {
    pretendDesktop();
    pretendAgent(AGENTS.webView2);
    expect(isMacOS()).toBe(false);
  });

  it("does not mistake Linux's WebKitGTK for macOS, though it is a WebKit too", () => {
    // The trap: WebKitGTK's agent carries `AppleWebKit` and `Safari` like WKWebView's does, and
    // differs only in the platform token — a looser test than `Mac` would pass here wrongly.
    pretendDesktop();
    pretendAgent(AGENTS.webKitGtk);
    expect(isMacOS()).toBe(false);
  });

  it("answers false in a browser on a Mac, where the question is meaningless", () => {
    // No Tauri global: this is the web build, the browser draws its own chrome, and nothing in the
    // page may reserve room for window buttons however Apple-shaped the host is.
    pretendAgent(AGENTS.wkWebView);
    expect(isMacOS()).toBe(false);
  });
});
