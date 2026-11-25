// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

// Coverage matrix (VATUSA/OIS#1005)
//
// Requirement                                          | Test                                                                  | Status
// -----------------------------------------------------|-----------------------------------------------------------------------|-------
// AC1/AC3 macOS control, expanded pill                 | navigator.platform "MacIntel": kbd reads "⌘K"                         | done
// AC1/AC3 macOS control, collapsed button              | navigator.platform "MacIntel": aria-label "Search (⌘K)"               | done
// AC1/AC3 Windows browser, expanded pill               | navigator.platform "Win32": kbd reads "Ctrl K", no ⌘ in the pill      | done
// AC1/AC3 Windows browser, collapsed button            | navigator.platform "Win32": aria-label "Search (Ctrl K)"              | done
// AC1 Linux (browser and desktop WebKitGTK)            | navigator.platform "Linux x86_64": "Ctrl K" / "Search (Ctrl K)"       | done
// AC1 desktop app on Windows (WebView2 uaData shape)   | userAgentData {platform:"Windows"} + platform "MacIntel": "Ctrl K"    | done
// AC1 interaction still works with the new label       | clicking the expanded pill / collapsed button opens command search    | done
//
// Mocks and renderSidebar(collapsed) mirror app-sidebar.window-chrome.test.tsx (router, @/lib/auth,
// @tauri-apps/api/window, matchMedia stub, Theme/Toast/Tooltip providers, act + unmount in afterEach).
// The platform is read at module load, so each case: stub navigator (Object.defineProperty on
// navigator.platform / navigator.userAgentData, configurable) FIRST, then vi.resetModules() and
// `await import("./app-sidebar")`; restore both properties in afterEach.
// Query the pill as the expanded button whose text contains "Search" and its <kbd>; the collapsed
// control by button[aria-label^="Search"]. Every Ctrl case asserts the exact "Ctrl K" text, which the
// pre-fix "⌘K" fails, beside the macOS control that expects "⌘K".

vi.mock("@tauri-apps/api/window", () => ({getCurrentWindow: () => ({label: "main"})}));
vi.mock("@tanstack/react-router", () => ({
  Link: React.forwardRef<HTMLAnchorElement, {to: string; children?: React.ReactNode}>(({to, children}, ref) => (
    <a ref={ref} href={to}>
      {children}
    </a>
  )),
  useRouter: () => ({history: {back: () => undefined, forward: () => undefined}}),
  useRouterState: () => "/",
}));
vi.mock("@/lib/auth", () => ({
  useMe: () => ({data: {id: "u1", display_name: "Test User", cid: 1, permissions: []}}),
  useLogout: () => ({mutate: () => undefined}),
  useLogin: () => ({mutate: () => undefined}),
  useSignInPending: () => false,
}));
const search = vi.hoisted(() => ({open: vi.fn()}));
vi.mock("./command-search", () => ({openCommandSearch: search.open}));

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  window.matchMedia ??= ((query: string) =>
    ({matches: false, media: query, addEventListener() {}, removeEventListener() {}}) as unknown as MediaQueryList);
});

const originalPlatform = navigator.platform;
let root: ReturnType<typeof createRoot> | undefined;
afterEach(() => {
  act(() => root?.unmount());
  root = undefined;
  document.body.innerHTML = "";
  Object.defineProperty(navigator, "platform", {value: originalPlatform, configurable: true});
  delete (navigator as {userAgentData?: unknown}).userAgentData;
  search.open.mockClear();
  vi.resetModules();
});

/**
 * Sets the host the helper reads at module load, then imports a fresh sidebar (and the `@ois/ui`
 * providers it shares context with) so the read sees this host, and renders it.
 */
async function renderOn(
  host: {platform: string; userAgentData?: {platform: string}},
  collapsed: boolean,
): Promise<HTMLElement> {
  Object.defineProperty(navigator, "platform", {value: host.platform, configurable: true});
  if (host.userAgentData) {
    Object.defineProperty(navigator, "userAgentData", {value: host.userAgentData, configurable: true});
  }
  vi.resetModules();
  const ui = await import("@ois/ui");
  const {AppSidebar} = await import("./app-sidebar");
  const el = document.createElement("div");
  document.body.append(el);
  root = createRoot(el);
  await act(async () => {
    root!.render(
      <ui.ThemeProvider>
        <ui.ToastProvider>
          <ui.TooltipProvider>
            <AppSidebar collapsed={collapsed} onToggle={() => undefined} />
          </ui.TooltipProvider>
        </ui.ToastProvider>
      </ui.ThemeProvider>,
    );
  });
  return el;
}

/** The expanded sidebar's search pill: the button whose text starts with "Search". */
function pill(el: HTMLElement): HTMLButtonElement {
  const found = [...el.querySelectorAll("button")].find((b) => b.textContent?.trim().startsWith("Search"));
  expect(found, "the expanded search pill").toBeTruthy();
  return found!;
}

/** The collapsed sidebar's search button, found by its label's prefix whatever key it names. */
function collapsedSearch(el: HTMLElement): HTMLButtonElement {
  const found = el.querySelector<HTMLButtonElement>('button[aria-label^="Search ("]');
  expect(found, "the collapsed search button").toBeTruthy();
  return found!;
}

describe("the sidebar search hint on macOS (VATUSA/OIS#1005 AC3 control)", () => {
  it("shows '⌘K' in the expanded search pill's kbd when navigator.platform is 'MacIntel'", async () => {
    const el = await renderOn({platform: "MacIntel"}, false);
    expect(pill(el).querySelector("kbd")?.textContent).toBe("⌘K");
  });
  it("labels the collapsed search button 'Search (⌘K)' when navigator.platform is 'MacIntel'", async () => {
    const el = await renderOn({platform: "MacIntel"}, true);
    expect(collapsedSearch(el).getAttribute("aria-label")).toBe("Search (⌘K)");
  });
});

describe("the sidebar search hint on Windows (VATUSA/OIS#1005 AC1, AC3)", () => {
  it("shows 'Ctrl K' and no ⌘ in the expanded search pill's kbd when navigator.platform is 'Win32'", async () => {
    const el = await renderOn({platform: "Win32"}, false);
    expect(pill(el).querySelector("kbd")?.textContent).toBe("Ctrl K");
    expect(pill(el).textContent).not.toContain("⌘");
  });
  it("labels the collapsed search button 'Search (Ctrl K)' when navigator.platform is 'Win32'", async () => {
    const el = await renderOn({platform: "Win32"}, true);
    expect(collapsedSearch(el).getAttribute("aria-label")).toBe("Search (Ctrl K)");
  });
});

describe("the sidebar search hint on Linux (VATUSA/OIS#1005 AC1)", () => {
  it("shows 'Ctrl K' in the expanded search pill's kbd when navigator.platform is 'Linux x86_64'", async () => {
    const el = await renderOn({platform: "Linux x86_64"}, false);
    expect(pill(el).querySelector("kbd")?.textContent).toBe("Ctrl K");
  });
  it("labels the collapsed search button 'Search (Ctrl K)' when navigator.platform is 'Linux x86_64'", async () => {
    const el = await renderOn({platform: "Linux x86_64"}, true);
    expect(collapsedSearch(el).getAttribute("aria-label")).toBe("Search (Ctrl K)");
  });
});

describe("the sidebar search hint in the Windows desktop app (VATUSA/OIS#1005 AC1)", () => {
  // WebView2 is Chromium: userAgentData answers. navigator.platform is set to 'MacIntel' so the case
  // goes red if the helper reads navigator.platform first.
  it("shows 'Ctrl K' in the pill and 'Search (Ctrl K)' on the collapsed button when userAgentData.platform is 'Windows'", async () => {
    const host = {platform: "MacIntel", userAgentData: {platform: "Windows"}};
    const expanded = await renderOn(host, false);
    expect(pill(expanded).querySelector("kbd")?.textContent).toBe("Ctrl K");
    act(() => root?.unmount());
    const collapsed = await renderOn(host, true);
    expect(collapsedSearch(collapsed).getAttribute("aria-label")).toBe("Search (Ctrl K)");
  });
});

describe("the search controls still open the palette (VATUSA/OIS#1005 AC1)", () => {
  it("opens command search when the expanded pill labeled 'Ctrl K' is clicked", async () => {
    const el = await renderOn({platform: "Win32"}, false);
    act(() => pill(el).click());
    expect(search.open).toHaveBeenCalledTimes(1);
  });
  it("opens command search when the collapsed 'Search (Ctrl K)' button is clicked", async () => {
    const el = await renderOn({platform: "Win32"}, true);
    act(() => collapsedSearch(el).click());
    expect(search.open).toHaveBeenCalledTimes(1);
  });
});
