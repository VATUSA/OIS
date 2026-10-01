// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it} from "vitest";

import {AdvisoriesTab} from "./advisories";
import type {Advisory} from "@/lib/advisories";
import type {Me} from "@/lib/auth";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

const roots: { root: ReturnType<typeof createRoot>; host: HTMLElement }[] = [];
afterEach(() => {
  for (const { root, host } of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
  document.body.innerHTML = "";
});

/**
 * A TMU controller holding read + create but **not** publish. Deliberately not `server_admin`, which
 * short-circuits `hasPermission` and would make every control render for the wrong reason — masking
 * exactly the gating this file is here to check.
 */
function me(actions: string[]): Me {
  return {
    id: "u1",
    cid: 1,
    email: "",
    display_name: "Test Controller",
    rating: null,
    primary_role: null,
    server_admin: false,
    permissions: { tmu: { adv: actions } },
  } as unknown as Me;
}

const DRAFT: Advisory = {
  id: "a1",
  facility: "DCC",
  kind: "reroute",
  number: 7,
  issued_day: "2026-09-30",
  status: "draft",
  body: "vATCSCC ADVZY 007 DCC 09/30/2026 CAMRN ARRIVALS\nORIG  DEST  ROUTE",
  created_at: "2026-09-30T12:00:00Z",
};

/** Mount with the query cache pre-seeded — never stub `fetch`. */
function mount(actions: string[], advisories: Advisory[] = [DRAFT]) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  qc.setQueryData(["advisories"], advisories);
  qc.setQueryData(["me"], me(actions));

  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({ root, host });
  act(() =>
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <AdvisoriesTab />
        </ToastProvider>
      </QueryClientProvider>,
    ),
  );
  return host;
}

const buttons = (host: HTMLElement) => [...host.querySelectorAll("button")].map((b) => b.textContent ?? "");
const click = (el: Element) => act(() => el.dispatchEvent(new MouseEvent("click", { bubbles: true })));

describe("AdvisoriesTab gating (VATUSA/OIS#460 AC3)", () => {
  it("offers no publish control to a holder without tmu.adv.publish", () => {
    const host = mount(["read", "create"]);
    expect(buttons(host).some((t) => t.includes("Publish"))).toBe(false);
  });

  it("offers publish to a holder of tmu.adv.publish", () => {
    const host = mount(["read", "publish"]);
    expect(buttons(host).some((t) => t.includes("Publish"))).toBe(true);
  });

  it("offers no create form without tmu.adv.create", () => {
    const host = mount(["read"]);
    expect(host.querySelector('[aria-label="Entry mode"]')).toBeNull();
  });

  it("offers the create form with tmu.adv.create", () => {
    const host = mount(["read", "create"]);
    expect(host.querySelector('[aria-label="Entry mode"]')).not.toBeNull();
  });

  it("offers cancel only on a published advisory", () => {
    expect(buttons(mount(["read", "publish"], [DRAFT])).some((t) => t.includes("Cancel"))).toBe(false);
    const published = mount(["read", "publish"], [{ ...DRAFT, status: "published" }]);
    expect(buttons(published).some((t) => t.includes("Cancel"))).toBe(true);
    // ...and publish is gone once it is published — a document goes out once.
    expect(buttons(published).some((t) => t.includes("Publish"))).toBe(false);
  });
});

describe("AdvisoriesTab preview (VATUSA/OIS#460 AC2)", () => {
  it("shows the body the API returned, verbatim", () => {
    // The assertion that keeps a TypeScript renderer from creeping back in: the preview is whatever
    // `body` the backend derived from `structured`, so it is the document that will actually post.
    // Rendering it locally would be a second implementation of `advisory.rs` (#455).
    const host = mount(["read", "create"]);
    click([...host.querySelectorAll("button")].find((b) => b.textContent?.includes("Preview"))!);
    const pre = host.querySelector('[aria-label="Rendered advisory"]');
    expect(pre).not.toBeNull();
    expect(pre!.textContent).toBe(DRAFT.body);
  });

  it("shows no preview until one is asked for", () => {
    const host = mount(["read", "create"]);
    expect(host.querySelector('[aria-label="Rendered advisory"]')).toBeNull();
  });
});

describe("AdvisoriesTab entry mode (VATUSA/OIS#460 AC1)", () => {
  it("switches between structured and raw, the same toggle as TMIs", () => {
    const host = mount(["read", "create"]);
    // Structured is the default: the route fields are present, the raw textarea is not.
    expect(host.querySelector('[aria-label="Row 1 route"]')).not.toBeNull();
    expect(host.querySelector('[aria-label="Document text"]')).toBeNull();

    click([...host.querySelectorAll("button")].find((b) => b.textContent?.trim() === "Raw")!);
    expect(host.querySelector('[aria-label="Document text"]')).not.toBeNull();
    expect(host.querySelector('[aria-label="Row 1 route"]')).toBeNull();
  });
});
