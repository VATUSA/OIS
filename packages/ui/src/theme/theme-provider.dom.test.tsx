// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

import {ThemeProvider, useTheme} from "./theme-provider";

/**
 * ThemeProvider mounts above the app's ErrorBoundary, so a storage read that throws would leave a
 * blank page (#947). Every case mounts with defaultTheme="dark" so the fallback is distinguishable
 * from the built-in "system" default, which resolves to "light" under the matchMedia stub below.
 */
declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  // ThemeProvider reads the OS color scheme, and jsdom has no `matchMedia`.
  window.matchMedia ??= ((query: string) =>
    ({matches: false, media: query, addEventListener() {}, removeEventListener() {}}) as unknown as MediaQueryList);
});

let root: ReturnType<typeof createRoot> | undefined;
let host: HTMLElement | undefined;
afterEach(() => {
  act(() => root?.unmount());
  root = undefined;
  host?.remove();
  host = undefined;
  vi.restoreAllMocks();
  localStorage.clear();
  document.documentElement.className = "";
});

let current: ReturnType<typeof useTheme> | undefined;
function Probe() {
  current = useTheme();
  return <span data-testid="probe">{current.theme}</span>;
}

function mount(defaultTheme: "light" | "dark" | "system" = "dark") {
  current = undefined;
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() =>
    root!.render(
      <ThemeProvider defaultTheme={defaultTheme}>
        <Probe />
      </ThemeProvider>,
    ),
  );
}

describe("ThemeProvider storage guards", () => {
  it("falls back to the default theme when getItem throws", () => {
    const getItem = vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new DOMException("blocked", "SecurityError");
    });

    mount();

    expect(getItem).toHaveBeenCalledWith("ois-theme");
    expect(document.querySelector('[data-testid="probe"]')?.textContent).toBe("dark");
    expect(current?.theme).toBe("dark");
    expect(current?.resolvedTheme).toBe("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
  });

  it("falls back to the default theme for an unknown stored value", () => {
    localStorage.setItem("ois-theme", "purple");

    mount();

    expect(current?.theme).toBe("dark");
    expect(current?.resolvedTheme).toBe("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
  });

  it("applies a valid stored theme over the default", () => {
    localStorage.setItem("ois-theme", "light");

    mount();

    expect(current?.theme).toBe("light");
    expect(current?.resolvedTheme).toBe("light");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });

  it("accepts each of light, dark and system from storage", () => {
    for (const stored of ["light", "dark", "system"] as const) {
      const fallback = stored === "dark" ? "light" : "dark";
      localStorage.setItem("ois-theme", stored);

      mount(fallback);

      expect(current?.theme, `stored "${stored}" over default "${fallback}"`).toBe(stored);
      act(() => root?.unmount());
      root = undefined;
      host?.remove();
      host = undefined;
    }
  });

  it("switches theme when setItem throws, without throwing", () => {
    mount();
    expect(current?.theme).toBe("dark");
    const setItem = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new DOMException("quota", "QuotaExceededError");
    });

    let thrown: unknown;
    act(() => {
      try {
        current!.setTheme("light");
      } catch (e) {
        thrown = e;
      }
    });

    expect(thrown).toBeUndefined();
    expect(setItem).toHaveBeenCalledWith("ois-theme", "light");
    expect(current?.theme).toBe("light");
    expect(current?.resolvedTheme).toBe("light");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });
});
