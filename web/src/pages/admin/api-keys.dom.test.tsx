// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

const del = vi.hoisted(() => vi.fn());
const put = vi.hoisted(() => vi.fn());

// The generated client captures `fetch` when its module loads, so stubbing `fetch` is always too
// late (VATUSA/OIS#387): mock the client itself, and seed the cache so nothing loads.
vi.mock("@/lib/api", () => ({ois: {GET: vi.fn(), POST: vi.fn(), PUT: put, DELETE: del}}));
vi.mock("@/components/shell/page-meta", () => ({usePageHeader: () => {}}));

import {AdminApiKeys} from "./api-keys";
import type {ApiKey} from "@/lib/api-keys";
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
  put.mockReset();
});

// A real permission tree, not `server_admin`: `api_keys.key.delete` is what shows the Delete button
// and makes the limit cell an input.
const ME = {
  id: "u1",
  cid: 1,
  email: "",
  display_name: "Test Admin",
  rating: null,
  primary_role: null,
  server_admin: false,
  permissions: {api_keys: {key: ["read", "delete"]}},
} as unknown as Me;

const KEY: ApiKey = {
  id: "k1",
  name: "Ops script",
  prefix: "ois_abc",
  status: "active",
  permissions: [],
  created_at: "2026-09-01T00:00:00Z",
  owner_cid: 1000001,
  owner_display_name: "Some Controller",
  rate_limit_per_min: 60,
  usage: {requests_this_hour: 0, requests_last_day: 0, refused_last_day: 0},
};

// `useAllKeys(undefined)` reads ["admin-api-keys", null] (`web/src/lib/api-keys.ts:17,162`).
const KEYS = ["admin-api-keys", null];

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
  qc.setQueryData(KEYS, [KEY]);
  qc.setQueryData(["me"], ME);
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <AdminApiKeys />
        </ToastProvider>
      </QueryClientProvider>,
    );
  });
  return {host, qc};
}

/** Re-queried on every call: a remounted cell replaces the DOM node, so a held reference would lie. */
function deleteButton(host: HTMLElement) {
  const el = [...host.querySelectorAll("button")].find((b) => b.textContent?.trim() === "Delete");
  if (!el) throw new Error("no Delete button");
  return el;
}
const click = (el: Element) => act(async () => el.dispatchEvent(new MouseEvent("click", {bubbles: true})));
const toasts = () => [...document.querySelectorAll('[role="status"]')].map((t) => t.textContent ?? "");
/** The query cache notifies observers on a `setTimeout(0)`, which `act` alone does not wait for. */
const settle = () => act(() => new Promise<void>((r) => setTimeout(r, 0)));

describe("Deleting a key from the admin table confirms on the second click (#824)", () => {
  // Same failure as the advisories tab: the columns depended on the whole rate-limit mutation
  // result, so the warning toast's re-render rebuilt them and remounted the armed button.
  it("sends the DELETE after the warning toast has rendered", async () => {
    del.mockResolvedValue({data: undefined, error: undefined, response: {status: 204}});
    const {host} = await mount();

    await click(deleteButton(host));
    expect(toasts().some((t) => t.includes("Delete this key permanently?"))).toBe(true);
    expect(del).not.toHaveBeenCalled();

    await click(deleteButton(host));
    expect(del).toHaveBeenCalledTimes(1);
    expect(del).toHaveBeenCalledWith("/api/v1/admin/api-keys/{id}", {params: {path: {id: "k1"}}});
  });

  it("sends the DELETE when the key list refetches between the two clicks", async () => {
    del.mockResolvedValue({data: undefined, error: undefined, response: {status: 204}});
    const {host, qc} = await mount();

    await click(deleteButton(host));
    await act(async () => {
      // A changed row, not an equal copy: the query cache keeps the old reference for equal data,
      // and then nothing re-renders.
      qc.setQueryData(KEYS, [{...KEY, name: "Ops script v2"}]);
    });
    await settle();
    // Positive control: the page re-rendered with the refetched row before the second click.
    expect(host.querySelector("tbody")?.textContent).toContain("Ops script v2");
    await click(deleteButton(host));
    expect(del).toHaveBeenCalledTimes(1);
  });
});

describe("A rate limit typed into an admin key row is saved on blur (#824)", () => {
  // The rate-limit save now calls `useAdminSetKeyRateLimit().mutate` from the column, not the whole
  // mutation result, so this pins the argument shape the column passes.
  it("sends the typed limit for that key when the cell loses focus", async () => {
    put.mockResolvedValue({data: undefined, error: undefined, response: {status: 204}});
    await mount();
    const input = document.querySelector<HTMLInputElement>('input[aria-label="Ops script rate limit per minute"]');
    if (!input) throw new Error("no rate-limit input for the Ops script row");
    expect(input.value).toBe("60");

    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "45");
      input.dispatchEvent(new Event("input", {bubbles: true}));
    });
    await act(async () => input.dispatchEvent(new FocusEvent("focusout", {bubbles: true})));

    expect(put).toHaveBeenCalledTimes(1);
    expect(put).toHaveBeenCalledWith("/api/v1/admin/api-keys/{id}/rate-limit", {
      params: {path: {id: "k1"}},
      body: {rate_limit_per_min: 45},
    });
  });
});
