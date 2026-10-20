// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

const del = vi.hoisted(() => vi.fn());

// The generated client captures `fetch` when its module loads, so stubbing `fetch` is always too
// late (VATUSA/OIS#387): mock the client itself, and seed the cache so nothing loads.
vi.mock("@/lib/api", () => ({ois: {GET: vi.fn(), POST: vi.fn(), DELETE: del}}));

import {AdvisoriesTab} from "./advisories";
import type {Advisory} from "@/lib/advisories";
import type {Me} from "@/lib/auth";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

const roots: {root: ReturnType<typeof createRoot>; host: HTMLElement}[] = [];
afterEach(() => {
  for (const {root, host} of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
  document.body.innerHTML = "";
  del.mockReset();
});

const ME = {
  id: "u1",
  cid: 1,
  email: "",
  display_name: "Test Controller",
  rating: null,
  primary_role: null,
  server_admin: false,
  permissions: {tmu: {adv: ["read", "update"]}},
} as unknown as Me;

const DRAFT: Advisory = {
  id: "a1",
  facility: "DCC",
  kind: "reroute",
  number: 7,
  issued_day: "2026-09-30",
  status: "draft",
  body: "vATCSCC ADVZY 007 DCC 09/30/2026 CAMRN ARRIVALS",
  created_at: "2026-09-30T12:00:00Z",
};

async function mount() {
  const qc = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        staleTime: Infinity,
        refetchOnMount: false,
        refetchOnWindowFocus: false,
        refetchOnReconnect: false,
      },
    },
  });
  qc.setQueryData(["advisories"], [DRAFT]);
  qc.setQueryData(["me"], ME);
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <AdvisoriesTab />
        </ToastProvider>
      </QueryClientProvider>,
    );
  });
  return {host, qc};
}

/** Re-queried on every call: a remounted cell replaces the DOM node, so a held reference would lie. */
function discard(host: HTMLElement) {
  const el = [...host.querySelectorAll("button")].find((b) => b.textContent?.trim() === "Discard");
  if (!el) throw new Error("no Discard button");
  return el;
}
const click = (el: Element) => act(async () => el.dispatchEvent(new MouseEvent("click", {bubbles: true})));
const toasts = () => [...document.querySelectorAll('[role="status"]')].map((t) => t.textContent ?? "");
/** The query cache notifies observers on a `setTimeout(0)`, which `act` alone does not wait for. */
const settle = () => act(() => new Promise<void>((r) => setTimeout(r, 0)));

describe("Discard on a draft advisory confirms on the second click (#824)", () => {
  // The bug: the first click's warning toast re-rendered the tab, the columns rebuilt with a new
  // `cell` function, TanStack Table mounted it as a new component, and the armed button came back
  // idle. The second click armed it again and no DELETE was sent.
  it("sends the DELETE after the warning toast has rendered", async () => {
    del.mockResolvedValue({data: undefined, error: undefined, response: {status: 204}});
    const {host} = await mount();

    await click(discard(host));
    expect(toasts().some((t) => t.includes("Discard this draft?"))).toBe(true);
    expect(del).not.toHaveBeenCalled();

    await click(discard(host));
    expect(del).toHaveBeenCalledTimes(1);
    expect(del).toHaveBeenCalledWith("/api/v1/tmu/advisories/{id}", {params: {path: {id: "a1"}}});
    // One warning, not a second one from a re-armed button.
    expect(toasts().filter((t) => t.includes("Discard this draft?"))).toHaveLength(1);
  });

  // A refetch (realtime `tmu.advisory`, window focus) re-renders the tab without any toast. This pins
  // the columns' dependencies on their own, separately from the toast provider's memoized value.
  it("sends the DELETE when the list refetches between the two clicks", async () => {
    del.mockResolvedValue({data: undefined, error: undefined, response: {status: 204}});
    const {host, qc} = await mount();

    await click(discard(host));
    await act(async () => {
      // A changed row, not an equal copy: the query cache keeps the old reference for equal data,
      // and then nothing re-renders.
      qc.setQueryData(["advisories"], [{...DRAFT, facility: "ZNY"}]);
    });
    await settle();
    // Positive control: the tab re-rendered with the refetched row before the second click.
    expect(host.querySelector("tbody")?.textContent).toContain("ZNY");
    await click(discard(host));
    expect(del).toHaveBeenCalledTimes(1);
  });
});

describe("A failed discard says why (#824)", () => {
  async function failWith(status: number) {
    del.mockResolvedValue({data: undefined, error: {error: "x"}, response: {status}});
    const {host} = await mount();
    await click(discard(host));
    await click(discard(host));
    expect(del).toHaveBeenCalledTimes(1);
    // The last toast is the mutation's error; the first is the arming warning.
    return toasts().at(-1);
  }

  // AC3: a 403 (outside the caller's facility scope), a 404 and a 409 used to share one message.
  it.each([
    [403, "You can’t discard drafts for this facility"],
    [404, "That draft no longer exists"],
    [409, "Only a draft can be discarded"],
  ])("answers a %i with its own toast", async (status, message) => {
    expect(await failWith(status)).toContain(message);
  });

  it("falls back to a plain failure for any other status", async () => {
    expect(await failWith(500)).toContain("Couldn’t discard the draft");
  });
});
