// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

const get = vi.hoisted(() => vi.fn());
const post = vi.hoisted(() => vi.fn());

// The generated client captures `fetch` when its module loads, so mock the client itself (#387).
vi.mock("@/lib/api", () => ({ois: {GET: get, POST: post, PUT: vi.fn()}}));
vi.mock("@/components/shell/page-meta", () => ({usePageHeader: () => {}}));

import {AdminAccessControl} from "./access-control";

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
});

const preview = {
  dry_run: true,
  pull_summary: null,
  users_checked: 40,
  users_reset: 1,
  users: [
    {
      cid: 1795001,
      display_name: "Member",
      reattached: true,
      added: [{kind: "group", name: "EC", artcc_id: "ZJX", source: "vatusa", granted: true}],
      removed: [
        {kind: "group", name: "ACE", artcc_id: "ZDC", source: "manual", granted: true},
        {kind: "permission", name: "tmu.tmi.publish", artcc_id: null, source: "manual", granted: false},
      ],
    },
  ],
};

/** The real access page, signed in as a server admin or not; every read is answered by path. */
async function mount(serverAdmin: boolean) {
  get.mockImplementation(async (path: string) => {
    if (path === "/api/v1/me") return {data: {server_admin: serverAdmin}, error: undefined};
    if (path === "/api/v1/admin/users") {
      return {data: {items: [], total: 0, page: 1, page_size: 25}, error: undefined};
    }
    if (path === "/api/v1/access/catalog") {
      return {data: {roles: [], permissions: {}, facilities: []}, error: undefined};
    }
    if (path === "/api/v1/admin/access/vatusa-reset") return {data: preview, error: undefined};
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
  await vi.waitFor(() => expect(get).toHaveBeenCalledWith("/api/v1/me"));
  await act(async () => {});
  return host;
}

const resetButton = (host: HTMLElement) =>
  [...host.querySelectorAll("button")].find((b) => b.textContent?.includes("Reset all access to VATUSA"));

function type(label: string, value: string) {
  const input = document.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`)!;
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  act(() => {
    setter.call(input, value);
    input.dispatchEvent(new Event("input", {bubbles: true}));
  });
}

describe("Reset all access to VATUSA (VATUSA/OIS#795)", () => {
  it("is offered to the server admin only", async () => {
    const admin = await mount(true);
    await vi.waitFor(() => expect(resetButton(admin)).toBeDefined());
    // A fresh mount, so the wait below is for this mount's own `/me`.
    get.mockClear();
    const other = await mount(false);
    expect(other.querySelector('input[aria-label="Search users"]')).not.toBeNull();
    expect(resetButton(other)).toBeUndefined();
    expect(get).not.toHaveBeenCalledWith("/api/v1/admin/access/vatusa-reset");
  });

  it("shows the dry run and applies only with a reason and the confirmation word", async () => {
    post.mockResolvedValue({data: {...preview, dry_run: false, pull_summary: "ok"}, error: undefined});
    const host = await mount(true);
    const open = await vi.waitFor(() => {
      const b = resetButton(host);
      if (!b) throw new Error("no reset button yet");
      return b;
    });
    await act(async () => open.click());

    await vi.waitFor(() => expect(document.body.textContent).toContain("1 of 40 users would change"));
    const body = document.body.textContent ?? "";
    expect(body).toContain("Back on VATUSA sync");
    expect(body).toContain("− ACE · ZDC · manual");
    expect(body).toContain("− deny tmu.tmi.publish · national · manual");
    expect(body).toContain("+ EC · ZJX · vatusa");

    const apply = () => [...document.querySelectorAll("button")].find((b) => b.textContent === "Reset access")!;
    expect(apply().disabled).toBe(true);
    type("Confirmation", "RESET");
    expect(apply().disabled, "the word alone is not enough: a reason is required").toBe(true);
    type("Confirmation", "");
    type("Reason", "drift cleanup");
    expect(apply().disabled, "a reason alone is not enough").toBe(true);
    type("Confirmation", "reset");
    expect(apply().disabled, "the word is case-sensitive").toBe(true);
    type("Confirmation", "RESET");
    expect(apply().disabled).toBe(false);
    expect(post).not.toHaveBeenCalled();

    await act(async () => apply().click());
    expect(post).toHaveBeenCalledWith("/api/v1/admin/access/vatusa-reset", {body: {reason: "drift cleanup"}});
  });

  it("cannot apply without a dry run to show", async () => {
    const host = await mount(true);
    get.mockImplementation(async (path: string) =>
      path === "/api/v1/admin/access/vatusa-reset"
        ? {data: undefined, error: {status: 500}}
        : {data: {server_admin: true}, error: undefined},
    );
    const open = await vi.waitFor(() => {
      const b = resetButton(host);
      if (!b) throw new Error("no reset button yet");
      return b;
    });
    await act(async () => open.click());
    await vi.waitFor(() => expect(document.body.textContent).toContain("Couldn't load the dry run."));
    type("Reason", "drift cleanup");
    type("Confirmation", "RESET");
    const apply = [...document.querySelectorAll("button")].find((b) => b.textContent === "Reset access")!;
    expect(apply.disabled).toBe(true);
  });
});
