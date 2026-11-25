import {afterEach, describe, expect, it, vi} from "vitest";

import {isApplePlatform, shortcutLabel} from "./shortcut-label";

// Coverage matrix (VATUSA/OIS#1005)
//
// Requirement                                   | Test                                                                 | Status
// ----------------------------------------------|----------------------------------------------------------------------|-------
// AC1/AC3 macOS platform value -> "⌘K"          | isApplePlatform: "macOS" / "MacIntel" / "iPad" each true             | done
// AC1/AC3 Windows platform value -> "Ctrl K"    | isApplePlatform: "Windows" / "Win32" / "Linux x86_64" each false     | done
// AC1 empty / unknown platform fails to Ctrl    | isApplePlatform("") is false; shortcutLabel("K", "") is "Ctrl K"     | done
// AC2 one helper builds every label             | shortcutLabel("K", "MacIntel") / ("K", "Win32") literal outputs      | done
// AC2 key is passed through, not hard-coded "K" | shortcutLabel("P", ...) on both platforms                            | done
// AC1 desktop WebView2 (Chromium uaData)        | uaData "Windows" beats navigator.platform "MacIntel" -> "Ctrl K"     | done
// AC1 rule on two signals, other direction      | uaData "macOS" beats navigator.platform "Win32" -> "⌘K"              | done
// AC1 WebKit webviews (no uaData)               | navigator.platform used when userAgentData is absent                 | done
// AC1 uaData without a platform field           | falls through to navigator.platform                                  | done
// edge: no navigator (node / SSR)               | default label is "Ctrl K" and import does not throw                  | done
// edge: navigator with no platform fields      | default label is "Ctrl K"                                             | done
// plan: platform read once at module load       | a navigator change after import does not change the default label    | done
//
// Every expected value is a literal ("⌘K", "Ctrl K"), never computed by calling the helper.
// Module-load cases: vi.stubGlobal("navigator", …) first, then vi.resetModules() and
// `await import("./shortcut-label")`; vi.unstubAllGlobals() + vi.resetModules() in afterEach.

describe("isApplePlatform (VATUSA/OIS#1005 AC3)", () => {
  // Each entry driven on its own, failing entry named in the assertion message, so a dead
  // alternative in the regex fails alone rather than hiding behind "MacIntel".
  it("is true for each Apple platform value: 'macOS' (userAgentData), 'MacIntel' (navigator.platform), 'iPad', 'iPhone'", () => {
    for (const platform of ["macOS", "MacIntel", "iPad", "iPhone"]) {
      expect(isApplePlatform(platform), platform).toBe(true);
    }
  });
  it("is false for each non-Apple platform value: 'Windows' (userAgentData), 'Win32', 'Linux x86_64'", () => {
    for (const platform of ["Windows", "Win32", "Linux x86_64"]) {
      expect(isApplePlatform(platform), platform).toBe(false);
    }
  });
  it("is false for an empty platform string", () => {
    expect(isApplePlatform("")).toBe(false);
  });
});

describe("shortcutLabel with an explicit platform (VATUSA/OIS#1005 AC1, AC3)", () => {
  it("returns '⌘K' for the macOS platform value 'MacIntel'", () => {
    expect(shortcutLabel("K", "MacIntel")).toBe("⌘K");
  });
  it("returns 'Ctrl K' for the Windows platform value 'Win32'", () => {
    expect(shortcutLabel("K", "Win32")).toBe("Ctrl K");
  });
  it("returns 'Ctrl K' for 'Linux x86_64'", () => {
    expect(shortcutLabel("K", "Linux x86_64")).toBe("Ctrl K");
  });
  it("returns 'Ctrl K' for an empty platform, so an unknown host never shows ⌘", () => {
    expect(shortcutLabel("K", "")).toBe("Ctrl K");
  });
  it("passes the key through on both platforms: 'P' gives '⌘P' on 'macOS' and 'Ctrl P' on 'Windows'", () => {
    expect(shortcutLabel("P", "macOS")).toBe("⌘P");
    expect(shortcutLabel("P", "Windows")).toBe("Ctrl P");
  });
});

/** Imports a fresh copy of the module with `navigator` replaced, so its load-time read sees `nav`. */
async function loadWith(nav: unknown): Promise<typeof import("./shortcut-label")> {
  vi.stubGlobal("navigator", nav);
  vi.resetModules();
  return import("./shortcut-label");
}

describe("the platform read at module load (VATUSA/OIS#1005 AC1)", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.resetModules();
  });

  // Desktop WebView2 on Windows: Chromium answers through userAgentData.
  it("prefers userAgentData.platform 'Windows' over navigator.platform 'MacIntel', giving 'Ctrl K'", async () => {
    const m = await loadWith({userAgentData: {platform: "Windows"}, platform: "MacIntel"});
    expect(m.shortcutLabel("K")).toBe("Ctrl K");
  });
  // The other disagreement direction, so a predicate that still honors navigator.platform goes red.
  it("prefers userAgentData.platform 'macOS' over navigator.platform 'Win32', giving '⌘K'", async () => {
    const m = await loadWith({userAgentData: {platform: "macOS"}, platform: "Win32"});
    expect(m.shortcutLabel("K")).toBe("⌘K");
  });
  // WebKit (Safari, the macOS and Linux desktop webviews) has no userAgentData.
  it("uses navigator.platform 'MacIntel' when userAgentData is absent, giving '⌘K'", async () => {
    const m = await loadWith({platform: "MacIntel"});
    expect(m.shortcutLabel("K")).toBe("⌘K");
  });
  it("uses navigator.platform 'Linux x86_64' when userAgentData is absent, giving 'Ctrl K'", async () => {
    const m = await loadWith({platform: "Linux x86_64"});
    expect(m.shortcutLabel("K")).toBe("Ctrl K");
  });
  it("falls through to navigator.platform 'MacIntel' when userAgentData has no platform field, giving '⌘K'", async () => {
    const m = await loadWith({userAgentData: {}, platform: "MacIntel"});
    expect(m.shortcutLabel("K")).toBe("⌘K");
  });
  it("gives 'Ctrl K' when navigator has neither userAgentData nor platform", async () => {
    const m = await loadWith({});
    expect(m.shortcutLabel("K")).toBe("Ctrl K");
  });
  it("gives 'Ctrl K' when navigator is undefined, and importing the module does not throw", async () => {
    const m = await loadWith(undefined);
    expect(m.shortcutLabel("K")).toBe("Ctrl K");
  });
  it("keeps the platform it read at import: stubbing navigator.platform 'Win32' afterwards still gives '⌘K' from a 'MacIntel' import", async () => {
    const m = await loadWith({platform: "MacIntel"});
    vi.stubGlobal("navigator", {platform: "Win32"});
    expect(m.shortcutLabel("K")).toBe("⌘K");
  });
});
