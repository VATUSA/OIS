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

import {VatusaDetachedPill, VatusaSyncSection} from "./vatusa-sync-section";
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

const CID = 1549100;

/** A hand-managed user whose VATUSA roles now call for EC@ZDC and no longer support AEC@ZDC. */
const detached = {
  detached_at: "2026-10-04T12:00:00Z",
  detached_by: "Jane Admin",
  profile: {
    home_facility: "ZDC",
    rating_numeric: 5,
    home_controller: true,
    facility_join: null,
    synced_at: "2026-10-04T11:00:00Z",
    roles: [{facility: "ZDC", role: "DATM"}],
    visits: [],
  },
  resync_grants: [{group: "EC", artcc_id: "ZDC"}],
  resync_revokes: [{group: "AEC", artcc_id: "ZDC"}],
};

async function render(ui: React.ReactNode) {
  const qc = new QueryClient({defaultOptions: {queries: {retry: false}}});
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>{ui}</ToastProvider>
      </QueryClientProvider>,
    );
  });
  return host;
}

function answer(vatusa: object) {
  get.mockImplementation(async (path: string) =>
    path === "/api/v1/admin/users/{cid}/vatusa"
      ? {data: vatusa, error: undefined}
      : {data: undefined, error: {status: 404}},
  );
}

describe("VATUSA role sync in the access editor (VATUSA/OIS#549)", () => {
  it("shows a hand-managed user's state, who and when, and their VATUSA roles", async () => {
    answer(detached);
    const host = await render(
      <>
        <VatusaDetachedPill cid={CID} />
        <VatusaSyncSection cid={CID} />
      </>,
    );
    await vi.waitFor(() => expect(host.textContent).toContain("Manually managed"));
    expect(host.textContent).toContain("by Jane Admin");
    expect(host.textContent).toContain("DATM · ZDC");
  });

  it("previews exactly what a Resync will change, and won't confirm without a reason", async () => {
    answer(detached);
    const host = await render(<VatusaSyncSection cid={CID} />);
    const open = await vi.waitFor(() => {
      const b = [...host.querySelectorAll("button")].find((x) => x.textContent?.includes("Resync from VATUSA"));
      if (!b) throw new Error("no resync button yet");
      return b;
    });
    await act(async () => open.click());

    const body = document.body.textContent ?? "";
    expect(body).toContain("Will add");
    expect(body).toContain("EC · ZDC");
    expect(body).toContain("Will remove");
    expect(body).toContain("AEC · ZDC");
    const confirm = [...document.querySelectorAll("button")].find((b) => b.textContent === "Resync")!;
    expect(confirm.disabled).toBe(true);
    expect(post).not.toHaveBeenCalled();
  });

  it("offers no Resync, and shows no pill, while the user is synced", async () => {
    answer({...detached, detached_at: null, detached_by: null, resync_grants: [], resync_revokes: []});
    const host = await render(
      <>
        <VatusaDetachedPill cid={CID} />
        <VatusaSyncSection cid={CID} />
      </>,
    );
    await vi.waitFor(() => expect(host.textContent).toContain("DATM · ZDC"));
    expect(host.textContent).not.toContain("Manually managed");
    expect(host.textContent).not.toContain("Resync from VATUSA");
  });

  it("flags a hand-managed user in the user list", async () => {
    get.mockImplementation(async (path: string) => {
      if (path === "/api/v1/admin/users") {
        const items = [
          {cid: 1, display_name: "Synced", rating: null, roles: [], scoped_roles: [], vatusa_detached_at: null},
          {cid: 2, display_name: "Hand Managed", rating: null, roles: [], scoped_roles: [], vatusa_detached_at: "2026-10-04T12:00:00Z"},
        ];
        return {data: {items, total: 2, page: 1, page_size: 25}, error: undefined};
      }
      if (path === "/api/v1/access/catalog") {
        return {data: {roles: [], permissions: {}, facilities: []}, error: undefined};
      }
      return {data: undefined, error: {status: 404}};
    });
    const host = await render(<AdminAccessControl />);
    const row = (who: string) =>
      [...host.querySelectorAll("tbody tr")].find((r) => r.textContent?.includes(who))?.textContent ?? "";
    await vi.waitFor(() => expect(row("Hand Managed")).not.toBe(""));
    expect(row("Hand Managed")).toContain("Not synced");
    expect(row("Synced")).not.toContain("Not synced");
  });
});
