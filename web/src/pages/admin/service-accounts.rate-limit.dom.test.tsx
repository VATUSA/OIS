// @vitest-environment jsdom
// Coverage matrix (#824, AC6)
//
// Requirement                                 | Test                                                  | Red when
// --------------------------------------------|-------------------------------------------------------|--------------------------------------------
// AC6 half-typed limit survives a refetch     | keeps a half-typed rate limit when the accounts ...   | the columns memo depends on the whole
//                                             |                                                       | `useSetServiceAccountRateLimit()` result
// AC6 the kept draft is the one that commits  | (same test) blur saves the kept draft                 | RateLimitCell remounted and lost `draft`
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

const get = vi.hoisted(() => vi.fn());
const put = vi.hoisted(() => vi.fn());

// The generated client captures `fetch` when its module loads, so stubbing `fetch` is always too
// late (#387): mock the client itself, and seed the cache so nothing loads.
vi.mock("@/lib/api", () => ({ois: {GET: get, POST: vi.fn(), PUT: put, DELETE: vi.fn()}}));
vi.mock("@/components/shell/page-meta", () => ({usePageHeader: () => {}}));

import {AdminServiceAccounts} from "./service-accounts";
import type {Me} from "@/lib/auth";
import type {ServiceAccount} from "@/lib/service-accounts";

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
  get.mockReset();
  put.mockReset();
});

// A real permission tree, not `server_admin`: `service_accounts.update` is what makes the limit
// cell an input.
const ME = {
  id: "u1",
  cid: 1,
  email: "",
  display_name: "Test Admin",
  rating: null,
  primary_role: null,
  server_admin: false,
  permissions: {service_accounts: ["read", "update"]},
} as unknown as Me;

const ACCOUNT: ServiceAccount = {
  id: "sa1",
  key: "discord-bot",
  name: "Discord bot",
  description: null,
  status: "active",
  roles: ["BOT"],
  permissions: [],
  rate_limit_per_min: 60,
  usage: {requests_this_hour: 3, requests_last_day: 40, refused_last_day: 0},
  stale: false,
  last_used_at: null,
  expires_at: null,
  created_at: "2026-09-01T00:00:00Z",
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
  qc.setQueryData(["service-accounts"], [ACCOUNT]);
  qc.setQueryData(["me"], ME);
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <AdminServiceAccounts />
        </ToastProvider>
      </QueryClientProvider>,
    );
  });
  return {host, qc};
}

/** Re-queried on every call: a remounted cell replaces the DOM node, so a held reference would lie. */
function limitInput() {
  const el = document.querySelector<HTMLInputElement>('input[aria-label="Discord bot rate limit per minute"]');
  if (!el) throw new Error("no rate-limit input for the Discord bot row");
  return el;
}
/** React tracks a controlled input's value through the prototype setter, then reads `input` events. */
function type(el: HTMLInputElement, value: string) {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(el, value);
  el.dispatchEvent(new Event("input", {bubbles: true}));
}
/** The query cache notifies observers on a `setTimeout(0)`, which `act` alone does not wait for. */
const settle = () => act(() => new Promise<void>((r) => setTimeout(r, 0)));

describe("A rate limit typed into a service account row survives a refetch (#824)", () => {
  it("keeps a half-typed rate limit when the accounts list refetches, and saves it on blur", async () => {
    const {host, qc} = await mount();
    expect(limitInput().value).toBe("60");

    await act(async () => type(limitInput(), "45"));
    expect(limitInput().value).toBe("45");

    // A real refetch through the hook's queryFn: usage moved, the stored limit did not, so the cell's
    // `shown` value is unchanged and only a remount could reset the draft.
    get.mockResolvedValue({
      data: [{...ACCOUNT, usage: {requests_this_hour: 9, requests_last_day: 46, refused_last_day: 0}}],
      error: undefined,
      response: {status: 200},
    });
    await act(async () => {
      await qc.refetchQueries({queryKey: ["service-accounts"]});
    });
    await settle();
    // Positive control: the refetch landed and the page re-rendered before the draft is read.
    expect(get).toHaveBeenCalledWith("/api/v1/admin/service-accounts");
    expect(host.querySelector("tbody")?.textContent).toContain("9 this hour");

    expect(limitInput().value).toBe("45");
    expect(put).not.toHaveBeenCalled();

    // The kept draft is live state, not a stale DOM value: blurring commits it.
    put.mockResolvedValue({data: undefined, error: undefined, response: {status: 204}});
    await act(async () => limitInput().dispatchEvent(new FocusEvent("focusout", {bubbles: true})));
    expect(put).toHaveBeenCalledTimes(1);
    expect(put).toHaveBeenCalledWith("/api/v1/admin/service-accounts/{id}/rate-limit", {
      params: {path: {id: "sa1"}},
      body: {rate_limit_per_min: 45},
    });
  });
});
