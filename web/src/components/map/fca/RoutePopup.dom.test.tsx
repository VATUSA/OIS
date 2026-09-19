// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it} from "vitest";

import {RoutePopup} from "./RoutePopup";
import type {AircraftRoute} from "@/lib/fca";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}

beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  // jsdom has no layout engine; RoutePopup measures itself to stay on screen.
  Element.prototype.getBoundingClientRect = () =>
    ({ width: 320, height: 240, top: 0, left: 0, right: 320, bottom: 240 }) as DOMRect;
});

const roots: { root: ReturnType<typeof createRoot>; host: HTMLElement }[] = [];
afterEach(() => {
  for (const { root, host } of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
  document.body.innerHTML = "";
});

const ROUTE = {
  callsign: "BOGUS1",
  aircraft_type: "B738",
  dep: "KJFK",
  arr: "KBOS",
  points: [
    [40.6, -73.8],
    [42.4, -71.0],
  ],
  unresolved: [],
} as unknown as AircraftRoute;

/** Mounts the popup with the real query cache seeded, so nothing here is a mock of the wiring. */
async function mount(props: Partial<React.ComponentProps<typeof RoutePopup>>) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, refetchInterval: false } },
  });
  // `useSetting` reads the preferences blob; seed it so the popup renders settled.
  qc.setQueryData(["preferences", "settings"], {});

  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({ root, host });
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <RoutePopup
            route={ROUTE}
            fca={null}
            match={undefined}
            onClose={() => {}}
            {...props}
          />
        </ToastProvider>
      </QueryClientProvider>,
    );
  });
  return host;
}

const removeButton = (host: HTMLElement) =>
  host.querySelector('[aria-label="Remove BOGUS1 as a bogus flight"]');

describe("RoutePopup remove control (#342)", () => {
  it("offers the control to a controller who may edit the FCA", async () => {
    const host = await mount({ canEdit: true, onRemove: () => {} });
    expect(removeButton(host)).not.toBeNull();
  });

  // AC: "others without the permission don't see the control" — not rendered, not merely disabled
  // (DESIGN.md: controls the user can't use are not rendered).
  it("does not render the control without flow.fca.update", async () => {
    const host = await mount({ canEdit: false, onRemove: () => {} });
    expect(removeButton(host)).toBeNull();
  });

  // The exclusion is scoped to the FCA being worked, so with no FCA selected there is no facility
  // to record the removal against and the control must stay away.
  it("does not render the control when there is no FCA context", async () => {
    const host = await mount({ canEdit: true, onRemove: undefined });
    expect(removeButton(host)).toBeNull();
  });

  it("arms before firing, so a single click can't drop a flight", async () => {
    let removed: string | null = null;
    const host = await mount({
      canEdit: true,
      onRemove: (cs: string) => {
        removed = cs;
      },
    });
    const btn = removeButton(host) as HTMLButtonElement;
    await act(async () => {
      btn.click();
    });
    expect(removed).toBeNull();
    await act(async () => {
      (removeButton(host) as HTMLButtonElement).click();
    });
    expect(removed).toBe("BOGUS1");
  });
});
