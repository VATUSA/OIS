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
    post.mockResolvedValue({data: {run_id: "run-1"}, error: undefined, response: {status: 202}});
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

    // The run finishes at once, so no poll outlives the test.
    answerRun({...running, status: "succeeded", result: {...preview, dry_run: false, pull_summary: "ok"}});
    await act(async () => apply().click());
    expect(post).toHaveBeenCalledWith("/api/v1/admin/access/vatusa-reset", {body: {reason: "drift cleanup"}});
    await vi.waitFor(() => expect(document.body.textContent).toContain("1 of 40 users changed"));
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

  async function openDialog(host: HTMLElement) {
    const open = await vi.waitFor(() => {
      const b = resetButton(host);
      if (!b) throw new Error("no reset button yet");
      return b;
    });
    await act(async () => open.click());
    await vi.waitFor(() => expect(document.body.textContent).toContain("users would change"));
  }

  /** Answer the reset run's polls in turn, the last answer repeating. */
  function answerRun(...runs: object[]) {
    const base = get.getMockImplementation()!;
    const polls: string[] = [];
    get.mockImplementation(async (path: string, init?: {params?: {path?: {id?: string}}}) => {
      if (path !== "/api/v1/admin/access/vatusa-reset/runs/{id}") return base(path);
      polls.push(init?.params?.path?.id ?? "");
      return {data: runs[Math.min(polls.length, runs.length) - 1], error: undefined};
    });
    return polls;
  }

  async function apply(host: HTMLElement) {
    await openDialog(host);
    type("Reason", "drift cleanup");
    type("Confirmation", "RESET");
    const button = [...document.querySelectorAll("button")].find((b) => b.textContent === "Reset access")!;
    await act(async () => button.click());
  }

  const running = {id: "run-1", status: "running", started_at: "2026-10-08T12:00:00Z", finished_at: null, result: null, failure: null};

  it("waits for the run on the server and then shows its result (VATUSA/OIS#806)", async () => {
    post.mockResolvedValue({data: {run_id: "run-1"}, error: undefined, response: {status: 202}});
    const host = await mount(true);
    const polls = answerRun(running, {
      ...running,
      status: "succeeded",
      finished_at: "2026-10-08T12:03:00Z",
      result: {...preview, dry_run: false, pull_summary: "ok", users_reset: 2},
    });
    await apply(host);
    await vi.waitFor(() => expect(document.body.textContent).toContain("Resetting…"));
    expect(document.body.textContent).toContain("finishes even if you close this dialog");

    await vi.waitFor(() => expect(document.body.textContent).toContain("2 of 40 users changed"), {timeout: 4000});
    expect(polls).toEqual(["run-1", "run-1"]);
    expect(document.body.textContent).not.toContain("Resetting…");
  });

  it("shows why a run failed and how many users it reached", async () => {
    post.mockResolvedValue({data: {run_id: "run-1"}, error: undefined, response: {status: 202}});
    const host = await mount(true);
    answerRun({
      ...running,
      status: "failed",
      finished_at: "2026-10-08T12:01:00Z",
      failure: {error: "reset_incomplete", message: "the reset stopped part-way", users_reset: 3},
    });
    await apply(host);
    await vi.waitFor(() =>
      expect(document.body.textContent).toContain("the reset stopped part-way (3 users reset)"),
    );
  });

  it("shows why the server would not start a reset", async () => {
    post.mockResolvedValue({
      data: undefined,
      error: {error: "vatusa_not_configured", message: "VATUSA is not configured", users_reset: 0},
      response: {status: 503},
    });
    const host = await mount(true);
    const polls = answerRun(running);
    await apply(host);
    await vi.waitFor(() =>
      expect(document.body.textContent).toContain("VATUSA is not configured (0 users reset)"),
    );
    expect(polls).toEqual([]);
  });

  it("waits for the reset already running when the server refuses a second one", async () => {
    post.mockResolvedValue({data: undefined, error: {run_id: "run-0"}, response: {status: 409}});
    const host = await mount(true);
    const polls = answerRun({
      ...running,
      id: "run-0",
      status: "succeeded",
      finished_at: "2026-10-08T12:03:00Z",
      result: {...preview, dry_run: false, pull_summary: "ok", users_reset: 5},
    });
    await apply(host);
    await vi.waitFor(() => expect(document.body.textContent).toContain("5 of 40 users changed"));
    expect(polls).toEqual(["run-0"]);
  });

  it("runs a fresh dry run each time the dialog opens", async () => {
    const host = await mount(true);
    const dryRuns = () => get.mock.calls.filter(([path]) => path === "/api/v1/admin/access/vatusa-reset").length;
    await openDialog(host);
    expect(dryRuns()).toBe(1);
    const cancel = [...document.querySelectorAll("button")].find((b) => b.textContent === "Cancel")!;
    await act(async () => cancel.click());
    await openDialog(host);
    expect(dryRuns()).toBe(2);
  });
});
