// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {DialogProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it} from "vitest";

import {AddWidgetMenu} from "./AddWidgetMenu";
import type {Widget} from "./types";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}

beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  // Radix's menu primitives call pointer-capture APIs jsdom doesn't implement.
  const proto = Element.prototype as unknown as Record<string, unknown>;
  proto.hasPointerCapture ??= () => false;
  proto.setPointerCapture ??= () => {};
  proto.releasePointerCapture ??= () => {};
  proto.scrollIntoView ??= () => {};
});

const roots: { root: ReturnType<typeof createRoot>; host: HTMLElement }[] = [];
afterEach(() => {
  for (const { root, host } of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
  document.body.innerHTML = "";
});

/** A ZDC controller. Not `server_admin` — that would let the menu render for the wrong reason. */
const zdcController = {
  id: "u1",
  cid: 1,
  email: "a@b.c",
  display_name: "Tester",
  rating: null,
  server_admin: false,
  role_names: [],
  tmu_national: false,
  permissions: {},
  vatusa: { home_facility: "ZDC", visits: [] },
};

const nationalController = { ...zdcController, tmu_national: true };

/** Mounts the real menu against a seeded cache and opens it. */
async function openMenu(me: unknown) {
  const qc = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        refetchInterval: false,
        refetchOnMount: false,
        refetchOnWindowFocus: false,
        refetchOnReconnect: false,
        staleTime: Infinity,
      },
    },
  });
  qc.setQueryData(["me"], me);

  const added: Widget[] = [];
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({ root, host });
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <DialogProvider>
          <AddWidgetMenu onAdd={(w) => added.push(w)} />
        </DialogProvider>
      </QueryClientProvider>,
    );
  });

  const trigger = host.querySelector("button");
  if (!trigger) throw new Error("no trigger button rendered");
  await act(async () => {
    // Radix opens the menu on pointerdown, which jsdom does not synthesise from a click.
    trigger.dispatchEvent(
      new MouseEvent("pointerdown", { bubbles: true, cancelable: true, button: 0 }),
    );
    await new Promise((resolve) => setTimeout(resolve, 0));
  });

  // The content renders in a portal on document.body, not inside `host`.
  return { text: document.body.textContent ?? "", added };
}

describe("AddWidgetMenu national scope (VATUSA/OIS#474)", () => {
  it("offers the NAS section to a national reader", async () => {
    const { text } = await openMenu(nationalController);
    expect(text).toContain("National (NAS)");
  });

  it("does not offer it to a facility-scoped controller", async () => {
    const { text } = await openMenu(zdcController);
    // The facility ATC item still exists, so assert on the national section specifically.
    expect(text).toContain("Online ATC positions");
    expect(text).not.toContain("National (NAS)");
  });
});
