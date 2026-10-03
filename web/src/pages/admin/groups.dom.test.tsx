// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

const get = vi.hoisted(() => vi.fn());
const post = vi.hoisted(() => vi.fn());
const del = vi.hoisted(() => vi.fn());

// The generated client captures `fetch` when its module loads, so stubbing `fetch` is always too
// late (VATUSA/OIS#387) — mock the client itself, and seed the cache so nothing loads.
vi.mock("@/lib/api", () => ({ois: {GET: get, POST: post, DELETE: del}}));
vi.mock("@/components/shell/page-meta", () => ({usePageHeader: () => {}}));
vi.mock("@/lib/auth", () => ({useMe: () => ({data: {server_admin: true, permissions: {}}})}));

import {VatusaRoles} from "./groups";
import {type Group, VATUSA_ROLE_MAPPINGS, type VatusaRoleMappingList} from "@/lib/groups";

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
  post.mockReset();
  del.mockReset();
});

const ec: Group = {
  name: "EC",
  description: null,
  permissions: [],
  system: false,
  user_count: 0,
  service_account_count: 0,
};

const list: VatusaRoleMappingList = {
  mappings: [
    {id: 1, vatusa_role: "DATM", facility: null, role_name: "EC", created_at: "2026-10-01T00:00:00Z"},
    {id: 2, vatusa_role: "ATM", facility: "ZDC", role_name: "EC", created_at: "2026-10-01T00:00:00Z"},
    {id: 3, vatusa_role: "INS", facility: null, role_name: "NTMO", created_at: "2026-10-01T00:00:00Z"},
  ],
  known_vatusa_roles: ["ATM", "DATM", "INS", "WM"],
};

/** Sets a field through React's own value tracking (mirrors command-search.dom.test.tsx). */
function setValue(el: HTMLInputElement | HTMLSelectElement, value: string) {
  const proto = el instanceof HTMLSelectElement ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
  return act(async () => {
    Object.getOwnPropertyDescriptor(proto, "value")!.set!.call(el, value);
    el.dispatchEvent(new Event(el instanceof HTMLSelectElement ? "change" : "input", {bubbles: true}));
  });
}

function button(root: ParentNode, text: string) {
  const el = [...root.querySelectorAll("button")].find((b) => b.textContent?.trim() === text);
  if (!el) throw new Error(`no button labelled ${text}`);
  return el;
}

async function mount() {
  const qc = new QueryClient({
    defaultOptions: {queries: {retry: false, refetchOnMount: false, staleTime: Infinity}},
  });
  qc.setQueryData(VATUSA_ROLE_MAPPINGS, list);
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <VatusaRoles group={ec} facilities={[{id: "ZDC", name: "Washington"}]} />
        </ToastProvider>
      </QueryClientProvider>,
    );
  });
  return host;
}

describe("VATUSA roles on a group card (#548)", () => {
  it("lists only this group's mappings, with where each must be held", async () => {
    const host = await mount();
    expect(host.textContent).toContain("VATUSA roles — 2");
    // The rows, not the picker: INS is NTMO's mapping, though it is still offered as a choice.
    const rows = [...host.querySelectorAll("span.font-mono")].map((el) => el.textContent);
    expect(rows).toEqual(["DATM", "ATM"]);
    expect(host.textContent).toContain("any facility");
  });

  it("offers only VATUSA roles seen in synced members", async () => {
    const host = await mount();
    const [rolePicker] = host.querySelectorAll("select");
    const offered = [...rolePicker!.querySelectorAll("option")].map((o) => o.value).filter(Boolean);
    expect(offered).toEqual(["ATM", "DATM", "INS", "WM"]);
  });

  it("needs a role and a reason, then posts the mapping for this group", async () => {
    post.mockResolvedValue({
      data: {id: 4, vatusa_role: "WM", facility: "ZHQ", role_name: "EC", created_at: "x"},
      error: undefined,
      response: {status: 200},
    });
    get.mockResolvedValue({data: list, error: undefined});
    const host = await mount();
    const add = button(host, "Add VATUSA role");
    expect(add.disabled).toBe(true);

    const [rolePicker, facilityPicker] = host.querySelectorAll("select");
    await setValue(rolePicker!, "WM");
    await setValue(facilityPicker!, "ZHQ");
    expect(add.disabled).toBe(true); // still no reason
    await setValue(host.querySelector("input")!, "Division staff");
    expect(add.disabled).toBe(false);

    await act(async () => add.click());
    expect(post).toHaveBeenCalledWith("/api/v1/admin/vatusa-role-mappings", {
      body: {vatusa_role: "WM", facility: "ZHQ", role_name: "EC", reason: "Division staff"},
    });
  });
});
