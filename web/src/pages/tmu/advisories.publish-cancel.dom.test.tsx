// @vitest-environment jsdom
// Coverage matrix (#824, AC7)
//
// Requirement                              | Test                                                    | Red when
// -----------------------------------------|---------------------------------------------------------|---------------------------------------------
// AC7 publish 403/404/409 toast differently | answers a 403, a 404 and a 409 on publish with ...      | publish mutationFn drops `response.status`,
//                                          |                                                         | a status branch of its onError is removed, or
//                                          |                                                         | two of its messages are swapped
// AC7 cancel 403/404/409 toast differently  | answers a 403, a 404 and a 409 on cancel with ...       | the same, for useCancelAdvisory
// AC7 publish/cancel confirm on 2nd click   | sends the POST to publish / cancel on the second click  | the pre-fix code (toast context value and
//                                          |                                                         | columns together); the column deps alone are
//                                          |                                                         | pinned by the Discard refetch test in
//                                          |                                                         | advisories.confirm.dom.test.tsx
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

const post = vi.hoisted(() => vi.fn());

// The generated client captures `fetch` when its module loads, so stubbing `fetch` is always too
// late (#387): mock the client itself, and seed the cache so nothing loads.
vi.mock("@/lib/api", () => ({ois: {GET: vi.fn(), POST: post, DELETE: vi.fn()}}));

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
function unmountAll() {
  for (const {root, host} of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
  document.body.innerHTML = "";
}
afterEach(() => {
  unmountAll();
  post.mockReset();
});

// A real permission tree, not `server_admin`, which passes every `hasPermission` check on its own.
const ME = {
  id: "u1",
  cid: 1,
  email: "",
  display_name: "Test Controller",
  rating: null,
  primary_role: null,
  server_admin: false,
  permissions: {tmu: {adv: ["read", "update", "publish"]}},
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

const PUBLISHED: Advisory = {
  id: "a2",
  facility: "DCC",
  kind: "reroute",
  number: 8,
  issued_day: "2026-09-30",
  status: "published",
  body: "vATCSCC ADVZY 008 DCC 09/30/2026 PARCH ARRIVALS",
  created_at: "2026-09-30T13:00:00Z",
};

async function mount(rows: Advisory[]) {
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
  qc.setQueryData(["advisories"], rows);
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
function button(host: HTMLElement, label: string) {
  const el = [...host.querySelectorAll("tbody button")].find((b) => b.textContent?.trim() === label);
  if (!el) throw new Error(`no ${label} button in the table`);
  return el;
}
const click = (el: Element) => act(async () => el.dispatchEvent(new MouseEvent("click", {bubbles: true})));
const toasts = () => [...document.querySelectorAll('[role="status"]')].map((t) => t.textContent ?? "");
/** Lets the mutation's onError and the toast it pushes land before the toasts are read. */
const settle = () => act(() => new Promise<void>((r) => setTimeout(r, 0)));

describe("Publish and cancel confirm on the second click (#824)", () => {
  it("sends the POST to publish a draft on the second click", async () => {
    post.mockResolvedValue({data: {...DRAFT, status: "published"}, error: undefined, response: {status: 200}});
    const {host} = await mount([DRAFT]);

    await click(button(host, "Publish"));
    expect(toasts().some((t) => t.includes("Publish this advisory?"))).toBe(true);
    expect(post).not.toHaveBeenCalled();

    await click(button(host, "Publish"));
    expect(post).toHaveBeenCalledTimes(1);
    expect(post).toHaveBeenCalledWith("/api/v1/tmu/advisories/{id}/publish", {params: {path: {id: "a1"}}});
  });

  it("sends the POST to cancel a published advisory on the second click", async () => {
    post.mockResolvedValue({data: {...PUBLISHED, status: "cancelled"}, error: undefined, response: {status: 200}});
    const {host} = await mount([PUBLISHED]);

    await click(button(host, "Cancel"));
    expect(toasts().some((t) => t.includes("Cancel this advisory?"))).toBe(true);
    expect(post).not.toHaveBeenCalled();

    await click(button(host, "Cancel"));
    expect(post).toHaveBeenCalledTimes(1);
    expect(post).toHaveBeenCalledWith("/api/v1/tmu/advisories/{id}/cancel", {params: {path: {id: "a2"}}});
  });
});

describe("A failed publish or cancel says why (#824)", () => {
  /** Mounts a fresh tab, confirms `label` against a POST that answers `status`, and returns the
   *  error toast. Unmounts afterwards, so each status starts from an empty toast stack. */
  async function errorToastFor(row: Advisory, label: string, warn: string, status: number) {
    post.mockReset();
    post.mockResolvedValue({data: undefined, error: {error: "x"}, response: {status}});
    const {host} = await mount([row]);
    await click(button(host, label));
    await click(button(host, label));
    await settle();
    expect(post, `${label} ${status}: the POST was sent`).toHaveBeenCalledTimes(1);
    const shown = toasts().filter((t) => !t.includes(warn));
    unmountAll();
    expect(shown, `${label} ${status}: exactly one error toast`).toHaveLength(1);
    return shown[0]!;
  }

  // AC7 asks that each status reads differently, and differently from the fallback a 500 gets; the
  // exact strings pin which reason each status gets, so swapping two messages turns this red too.
  it.each([
    [
      "publish",
      DRAFT,
      "Publish",
      "Publish this advisory?",
      {
        403: "You can’t publish advisories for this facility",
        404: "That advisory no longer exists",
        409: "Only a draft can be published",
        500: "Couldn’t publish the advisory",
      },
    ],
    [
      "cancel",
      PUBLISHED,
      "Cancel",
      "Cancel this advisory?",
      {
        403: "You can’t cancel advisories for this facility",
        404: "That advisory no longer exists",
        409: "That advisory is already cancelled",
        500: "Couldn’t cancel the advisory",
      },
    ],
  ] as const)(
    "answers a 403, a 404 and a 409 on %s with three different toasts, none of them the fallback",
    async (_name, row, label, warn, expected) => {
      const byStatus = new Map<number, string>();
      for (const status of [403, 404, 409, 500] as const) {
        const shown = await errorToastFor(row, label, warn, status);
        expect(shown, `${label} ${status}`).toContain(expected[status]);
        byStatus.set(status, shown);
      }
      expect(new Set(byStatus.values()).size, JSON.stringify([...byStatus])).toBe(4);
    },
  );
});
