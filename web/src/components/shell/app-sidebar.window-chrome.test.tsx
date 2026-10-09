// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {ThemeProvider, ToastProvider, TooltipProvider} from "@ois/ui";
import {afterEach, beforeAll, beforeEach, describe, expect, it, vi} from "vitest";

/**
 * The desktop window keeps each platform's own title bar (#796), so the sidebar's chrome row draws no
 * window buttons and reserves no room for any: Back is the row's first control on every platform, as
 * it is on the web build.
 *
 * Window chrome drawn by the app would be gated on "Tauri, the `main` window", possibly with a
 * user-agent check for the host OS, so this renders the real sidebar in exactly that situation, once
 * per host OS, with the real `@/lib/platform`.
 */
const win = vi.hoisted(() => ({
  label: "main",
  minimize: () => Promise.resolve(),
  toggleMaximize: () => Promise.resolve(),
  close: () => Promise.resolve(),
  isFocused: () => Promise.resolve(true),
  onFocusChanged: () => Promise.resolve(() => undefined),
}));
vi.mock("@tauri-apps/api/window", () => ({getCurrentWindow: () => win}));

vi.mock("@tanstack/react-router", () => ({
  Link: React.forwardRef<HTMLAnchorElement, {to: string; children?: React.ReactNode}>(({to, children}, ref) => (
    <a ref={ref} href={to}>
      {children}
    </a>
  )),
  useRouter: () => ({history: {back: () => undefined, forward: () => undefined}}),
}));
vi.mock("@/lib/auth", () => ({
  useMe: () => ({data: {id: "u1", display_name: "Test User", cid: 1, permissions: []}}),
  useLogout: () => ({mutate: () => undefined}),
  useLogin: () => ({mutate: () => undefined}),
  useSignInPending: () => false,
}));

import {isMainWindow} from "@/lib/platform";

import {AppSidebar} from "./app-sidebar";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  // The theme toggle reads the OS colour scheme, and jsdom has no `matchMedia`.
  window.matchMedia ??= ((query: string) =>
    ({matches: false, media: query, addEventListener() {}, removeEventListener() {}}) as unknown as MediaQueryList);
});

/** The real user agents of the three webviews Tauri runs the app in. */
const HOSTS = {
  macOS:
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15",
  Windows:
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
  Linux:
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15",
} as const;

const originalAgent = navigator.userAgent;
function pretendAgent(ua: string) {
  Object.defineProperty(navigator, "userAgent", {value: ua, configurable: true});
}

let root: ReturnType<typeof createRoot> | undefined;
beforeEach(() => {
  window.__TAURI_INTERNALS__ = {};
  win.label = "main";
});
afterEach(() => {
  act(() => root?.unmount());
  root = undefined;
  delete window.__TAURI_INTERNALS__;
  pretendAgent(originalAgent);
});

async function renderSidebar(collapsed: boolean): Promise<HTMLElement> {
  const host = document.createElement("div");
  root = createRoot(host);
  await act(async () => {
    root!.render(
      <ThemeProvider>
      <ToastProvider>
        <TooltipProvider>
          <AppSidebar collapsed={collapsed} onToggle={() => undefined} />
        </TooltipProvider>
      </ToastProvider>
      </ThemeProvider>,
    );
  });
  // The old gate resolved the window label through a dynamic import after mount, so give any such
  // effect time to land before asserting that nothing appeared.
  for (let i = 0; i < 5; i++) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  }
  return host;
}

/** Everything the app-drawn window chrome left in the DOM, by every name it went by. */
function appDrawnChrome(host: HTMLElement): string[] {
  const found: string[] = [];
  const selectors = [
    "[data-tauri-drag-region]",
    "[data-window-chrome-slot]",
    ".traffic-lights",
    ".traffic-light",
  ];
  for (const selector of selectors) {
    if (host.querySelector(selector)) found.push(selector);
  }
  for (const button of host.querySelectorAll("button")) {
    const label = button.getAttribute("aria-label") ?? "";
    if (/^(close|minimi[sz]e|maximi[sz]e|zoom|restore)$/i.test(label)) found.push(`button "${label}"`);
  }
  return found;
}

const firstButton = (host: HTMLElement) => host.querySelector("button")?.getAttribute("aria-label");

/**
 * Whether `label`'s button opens the sidebar header with nothing before it, at either level: the
 * chrome row is the header's first element and the button is the row's first. Any leading spacer
 * fails this whatever it is called. The old macOS reservation was an empty sized `<div>`, which
 * neither the button scan nor `appDrawnChrome` would see without an attribute on it.
 */
function leadsTheHeader(host: HTMLElement, label: string): boolean {
  const button = host.querySelector(`button[aria-label="${label}"]`);
  const row = button?.parentElement;
  return !!row && row.firstElementChild === button && row.parentElement?.firstElementChild === row;
}

describe.each(Object.entries(HOSTS))("the sidebar's chrome row in the desktop main window on %s", (_os, ua) => {
  beforeEach(() => pretendAgent(ua));

  it("is the main window, so this is the situation the replica was drawn in", async () => {
    // Positive control: without it, a broken Tauri stand-in would make every "no chrome" below pass.
    await expect(isMainWindow()).resolves.toBe(true);
  });

  it("draws no window buttons or drag region, and leads with Back", async () => {
    const host = await renderSidebar(false);

    expect(appDrawnChrome(host)).toEqual([]);
    expect(firstButton(host)).toBe("Back");
    expect(leadsTheHeader(host, "Back"), "nothing may sit before Back in the chrome row").toBe(true);
  });

  it("draws no window buttons when collapsed, and leads with Expand sidebar", async () => {
    const host = await renderSidebar(true);

    expect(appDrawnChrome(host)).toEqual([]);
    expect(firstButton(host)).toBe("Expand sidebar");
    expect(leadsTheHeader(host, "Expand sidebar"), "nothing may sit before the collapsed row").toBe(true);
  });
});

describe("the chrome detector", () => {
  /** The detector has to see the old replica, or the "no chrome" assertions above prove nothing. */
  it("flags the markup the #419 replica rendered", () => {
    const host = document.createElement("div");
    host.innerHTML = `
      <div data-window-chrome-slot="">
        <div class="traffic-lights">
          <button class="traffic-light" aria-label="Close" data-tauri-drag-region="false"></button>
          <button class="traffic-light" aria-label="Minimize"></button>
          <button class="traffic-light" aria-label="Zoom"></button>
        </div>
      </div>`;

    expect(appDrawnChrome(host)).toEqual([
      "[data-tauri-drag-region]",
      "[data-window-chrome-slot]",
      ".traffic-lights",
      ".traffic-light",
      'button "Close"',
      'button "Minimize"',
      'button "Zoom"',
    ]);
  });
});
