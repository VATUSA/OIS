// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

const get = vi.hoisted(() => vi.fn());

// The generated client captures `fetch` when its module loads, so stubbing `fetch` is always too
// late (VATUSA/OIS#387) — mock the client itself.
vi.mock("@/lib/api", () => ({ois: {GET: get, POST: vi.fn(), PUT: vi.fn()}}));
vi.mock("@/components/shell/page-meta", () => ({usePageHeader: () => {}}));

import {AdminAccessControl} from "./access-control";
import type {AdminUserRow} from "@/lib/access";

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
});

/** Two holders of the same group, one nationally and one at ZDC — the case #546 is about. */
const users: AdminUserRow[] = [
  {cid: 1000001, display_name: "National Holder", rating: null, roles: ["EC"], scoped_roles: ["EC"]},
  {cid: 1000002, display_name: "ZDC Holder", rating: null, roles: ["EC"], scoped_roles: ["EC:ZDC"]},
];

/** Mounts the real page; every read is answered by path, so nothing reaches the network. */
async function mount() {
  get.mockImplementation(async (path: string) => {
    if (path === "/api/v1/admin/users") {
      return {data: {items: users, total: users.length, page: 1, page_size: 25}, error: undefined};
    }
    if (path === "/api/v1/access/catalog") {
      return {data: {roles: ["EC"], permissions: {}, facilities: []}, error: undefined};
    }
    return {data: undefined, error: {status: 404}};
  });
  const qc = new QueryClient({defaultOptions: {queries: {retry: false}}});
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <AdminAccessControl />
        </ToastProvider>
      </QueryClientProvider>,
    );
  });
  // The two reads resolve on later ticks; wait for the table rather than guessing how many.
  await vi.waitFor(() => {
    if (!host.querySelector("tbody tr")) throw new Error("users not rendered yet");
  });
  return host;
}

/** The Roles cell's text for the row naming `who`. */
function rolesOf(host: HTMLElement, who: string): string {
  const row = [...host.querySelectorAll("tbody tr")].find((r) => r.textContent?.includes(who));
  if (!row) throw new Error(`no row for ${who}`);
  return [...row.querySelectorAll("td")].at(-1)!.textContent ?? "";
}

describe("AdminAccessControl role badges (VATUSA/OIS#546)", () => {
  // AC5: the bare `roles` field is identical for these two, so a column reading it cannot tell
  // them apart — which is the bug.
  it("distinguishes a national membership from a facility-scoped one", async () => {
    const host = await mount();
    const national = rolesOf(host, "National Holder");
    const scoped = rolesOf(host, "ZDC Holder");
    expect(national).toBe("EC · national");
    expect(scoped).toBe("EC · ZDC");
  });
});
