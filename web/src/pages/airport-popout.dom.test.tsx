// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

import type {Flow} from "@/lib/feed";

const openPopout = vi.fn(() => Promise.resolve(true));
vi.mock("@/lib/popout", async (original) => ({
  ...(await original<typeof import("@/lib/popout")>()),
  openPopout: (...args: unknown[]) => openPopout(...(args as [])),
}));

const {LadderView} = await import("@/pages/airport");

/**
 * The airport ladder's pop-out control (VATUSA/OIS#790): desktop only, and only where the airport
 * page asks for it. Mounts the real `LadderView` against a real QueryClient with the settings blob
 * seeded, so the real `useSetting` and `can()` run; only the window opening itself is stubbed.
 */

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
});

const roots: {root: ReturnType<typeof createRoot>; host: HTMLElement}[] = [];
afterEach(() => {
  for (const {root, host} of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
  delete window.__TAURI_INTERNALS__;
  openPopout.mockClear();
});

const flow: Flow = {
  icao: "KIAD",
  aar: 40,
  flights: [
    {
      callsign: "AAL1",
      status: "airborne",
      eta: new Date(Date.now() + 10 * 60_000).toISOString(),
      gate: "BEARR",
    },
  ],
} as unknown as Flow;

function mount(node: React.ReactNode) {
  const qc = new QueryClient({
    defaultOptions: {queries: {retry: false, refetchInterval: false, refetchOnMount: false, staleTime: Infinity}},
  });
  qc.setQueryData(["preferences", "settings"], {"ladder.style": "tgui"});
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  act(() => root.render(<QueryClientProvider client={qc}>{node}</QueryClientProvider>));
  return host;
}

const control = (host: HTMLElement) =>
  host.querySelector<HTMLButtonElement>('button[aria-label="Pop out into a floating window"]');

describe("the airport ladder's pop-out control (#790)", () => {
  it("on desktop, opens the airport-ladder pop-out for that ICAO", () => {
    window.__TAURI_INTERNALS__ = {};
    const host = mount(<LadderView flow={flow} popoutIcao="KIAD" />);
    // The TGUI ladder is what's on screen, and the control sits on it.
    expect(host.querySelector('[data-ladder="tgui"]')).not.toBeNull();
    const button = control(host);
    expect(button).not.toBeNull();

    act(() => button!.click());
    expect(openPopout).toHaveBeenCalledTimes(1);
    expect(openPopout).toHaveBeenCalledWith({
      id: "airport-KIAD",
      title: "KIAD · ladder",
      route: "/popout/airport/KIAD",
    });
  });

  it("on the web build, doesn't render", () => {
    const host = mount(<LadderView flow={flow} popoutIcao="KIAD" />);
    expect(host.querySelector('[data-ladder="tgui"]')).not.toBeNull();
    expect(control(host)).toBeNull();
  });

  // The pop-out window and the dashboard widget render this same ladder without `popoutIcao`.
  it("doesn't render where the caller didn't ask for it, even on desktop", () => {
    window.__TAURI_INTERNALS__ = {};
    const host = mount(<LadderView flow={flow} />);
    expect(host.querySelector('[data-ladder="tgui"]')).not.toBeNull();
    expect(control(host)).toBeNull();
  });
});
