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

import {AdminGroups, VatusaRoles} from "./groups";
import {GROUPS, type Group, VATUSA_ROLE_MAPPINGS, type VatusaRoleMappingList} from "@/lib/groups";

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
    {id: 1, vatusa_role: "DATM", facility: null, role_name: "EC", created_at: "2026-10-01T00:00:00Z", holders: 3},
    {id: 2, vatusa_role: "ATM", facility: "ZDC", role_name: "EC", created_at: "2026-10-01T00:00:00Z", holders: 0},
    {id: 3, vatusa_role: "INS", facility: null, role_name: "NTMO", created_at: "2026-10-01T00:00:00Z", holders: 1},
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

async function mount(mappings: VatusaRoleMappingList = list) {
  const qc = new QueryClient({
    defaultOptions: {queries: {retry: false, refetchOnMount: false, staleTime: Infinity}},
  });
  qc.setQueryData(VATUSA_ROLE_MAPPINGS, mappings);
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

  // #699: an empty mapping table means VATUSA grants nothing at all, and a mapping no synced member
  // holds grants nobody. Both used to look like the same blank list.
  it("says when no mapping exists anywhere, so VATUSA grants nothing", async () => {
    const host = await mount({mappings: [], known_vatusa_roles: ["EVENT_COORDINATOR"]});
    expect(host.textContent).toContain("VATUSA sync adds no access until a mapping exists");
  });

  it("does not say so once any mapping exists, even another group's", async () => {
    const host = await mount({mappings: [list.mappings[2]!], known_vatusa_roles: ["INS"]});
    expect(host.textContent).not.toContain("VATUSA sync adds no access");
    expect(host.textContent).toContain("No VATUSA role grants this group.");
  });

  it("shows how many members each mapping matches, and warns when it matches nobody", async () => {
    const host = await mount();
    expect(host.textContent).toContain("3 members");
    const warnings = host.textContent!.split("so it grants nobody").length - 1;
    expect(warnings).toBe(1);
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

// --- The page: collapsed cards and domains (VATUSA/OIS#681) ---

const zdcEc: Group = {...ec, permissions: ["tmu.adv.create"], user_count: 1};

const catalog = {
  permissions: {tmu: {adv: ["create", "publish"]}, events: {plan: ["read", "update"]}},
  facilities: [{id: "ZDC", name: "Washington"}],
};

async function mountPage() {
  const qc = new QueryClient({
    defaultOptions: {queries: {retry: false, refetchOnMount: false, staleTime: Infinity}},
  });
  qc.setQueryData(GROUPS, [zdcEc]);
  qc.setQueryData(["access-catalog"], catalog);
  qc.setQueryData([...GROUPS, "EC", "members", 1], {items: [], total: 1, page_size: 25});
  qc.setQueryData(VATUSA_ROLE_MAPPINGS, list);
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <AdminGroups />
        </ToastProvider>
      </QueryClientProvider>,
    );
  });
  return host;
}

const card = (host: ParentNode) => host.querySelector<HTMLButtonElement>("h2 button")!;
const filter = (host: ParentNode) =>
  host.querySelector<HTMLInputElement>('input[placeholder="Filter permissions…"]');
const checkbox = (host: ParentNode, name: string) =>
  [...host.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')].find(
    (c) => c.closest("label")?.textContent?.includes(name),
  );
const click = (el: HTMLElement) => act(async () => el.click());

/** True when `el` and every ancestor is rendered (the hidden card body is `hidden`). */
const shown = (el: Element | null | undefined) => !!el && !el.closest("[hidden]");

describe("the Groups page (#681)", () => {
  it("starts with every card collapsed", async () => {
    const host = await mountPage();
    expect(card(host).getAttribute("aria-expanded")).toBe("false");
    expect(shown(filter(host))).toBe(false);
  });

  it("opens a card onto collapsed domains, each counting what the group grants", async () => {
    const host = await mountPage();
    await click(card(host));
    expect(shown(filter(host))).toBe(true);
    const domains = [...host.querySelectorAll<HTMLButtonElement>("button[aria-expanded]")].filter(
      (b) => b !== card(host),
    );
    expect(domains.map((d) => d.textContent)).toEqual(["events", "tmu1"]);
    expect(domains.every((d) => d.getAttribute("aria-expanded") === "false")).toBe(true);
    expect(checkbox(host, "tmu.adv.create")).toBeUndefined();

    await click(domains[1]);
    expect(checkbox(host, "tmu.adv.create")!.checked).toBe(true);
    expect(checkbox(host, "tmu.adv.publish")!.checked).toBe(false);
    expect(checkbox(host, "events.plan.read"), "events stays shut").toBeUndefined();
  });

  it("filters, opening every matching domain, and says when nothing matches", async () => {
    const host = await mountPage();
    await click(card(host));
    await setValue(filter(host)!, "plan");
    expect(checkbox(host, "events.plan.read")).toBeDefined();
    expect(checkbox(host, "tmu.adv.create")).toBeUndefined();
    await setValue(filter(host)!, "nothing-like-this");
    expect(host.textContent).toContain("No matches.");
  });

  it("offers no ARTCC scope on a group's permissions", async () => {
    const host = await mountPage();
    await click(card(host));
    await setValue(filter(host)!, "tmu");
    await click(checkbox(host, "tmu.adv.publish")!);
    // The scope control the access tab renders under a checked permission must not appear: a role's
    // permissions carry no scope, its membership does (groups.tsx).
    const editor = filter(host)!.closest("div")!.parentElement!;
    expect([...editor.querySelectorAll("button")].map((b) => b.textContent)).not.toContain("National");
    expect(editor.textContent).not.toContain("pick at least one ARTCC");
    expect(editor.textContent).not.toContain("facility-scoped");
  });

  it("keeps an unsaved permission change through collapsing and reopening the card", async () => {
    const host = await mountPage();
    await click(card(host));
    await setValue(filter(host)!, "tmu");
    await click(checkbox(host, "tmu.adv.publish")!);
    expect(checkbox(host, "tmu.adv.publish")!.checked).toBe(true);

    await click(card(host));
    expect(shown(filter(host))).toBe(false);
    await click(card(host));
    if (!checkbox(host, "tmu.adv.publish")) await setValue(filter(host)!, "tmu");
    expect(checkbox(host, "tmu.adv.publish")!.checked).toBe(true);
    // Still a pending edit: Save enables once there's a reason.
    const reason = [...host.querySelectorAll("label")]
      .find((l) => l.textContent?.startsWith("Reason (recorded"))!
      .querySelector("input")!;
    await setValue(reason, "tidy up");
    expect(button(host, "Save").disabled).toBe(false);
  });
});
