// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

const put = vi.hoisted(() => vi.fn());
const get = vi.hoisted(() => vi.fn());
const post = vi.hoisted(() => vi.fn());
// The "New service account" button is handed to the shell header, not rendered by the page, so the
// test captures it instead of mocking it away — otherwise the create flow is unreachable.
const header = vi.hoisted(() => ({actions: null as React.ReactNode}));

// The generated client captures `fetch` when its module loads, so stubbing `fetch` in a test is
// always too late (VATUSA/OIS#387) — mock the client itself.
vi.mock("@/lib/api", () => ({ois: {GET: get, POST: post, PUT: put}}));
vi.mock("@/components/shell/page-meta", () => ({
  usePageHeader: (opts: {actions?: React.ReactNode}) => {
    header.actions = opts.actions;
  },
}));
vi.mock("@/lib/auth", () => ({useMe: () => ({data: {server_admin: true, permissions: {}}})}));

import {AdminServiceAccounts} from "./service-accounts";
import {ACCOUNTS, type ServiceAccount} from "@/lib/service-accounts";

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
  put.mockReset();
  get.mockReset();
  post.mockReset();
  header.actions = null;
});

const account: ServiceAccount = {
  id: "sa-1",
  key: "sa_abc123",
  name: "Discord bot",
  description: null,
  roles: [],
  permissions: [],
  status: "active",
  created_at: "2026-09-01T00:00:00Z",
  last_used_at: null,
  expires_at: "2026-11-30T00:00:00Z",
  stale: false,
};

/** Types into a field through React's own value tracking (mirrors command-search.dom.test.tsx). */
const type = (input: HTMLInputElement, text: string) =>
  act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, text);
    input.dispatchEvent(new Event("input", {bubbles: true}));
  });

const HeaderActions = () => <div data-testid="header-actions">{header.actions}</div>;

/** Mounts the page against a seeded cache — nothing reaches the network. */
async function mount(accounts: ServiceAccount[] = [account]) {
  const qc = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        refetchOnMount: false,
        refetchOnWindowFocus: false,
        staleTime: Infinity,
      },
    },
  });
  qc.setQueryData(ACCOUNTS, accounts);
  qc.setQueryData(["service-account-roles"], ["BOT", "SERVICE_APP"]);
  qc.setQueryData(["service-account-grantable"], [
    {permission: "flow.fca.update", national: false, artccs: ["ZDC"]},
  ]);
  qc.setQueryData(["facilities"], [{id: "ZDC", name: "Washington"}]);

  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <AdminServiceAccounts />
          <HeaderActions />
        </ToastProvider>
      </QueryClientProvider>,
    );
  });
  return host;
}

/** Clicks a button by label. `root` is scoped deliberately: the table's sortable column headers are
 *  buttons too, so a page-wide search for "Roles" finds the header, not the row action. */
function click(root: ParentNode, text: string) {
  const el = [...root.querySelectorAll("button")].find((b) => b.textContent?.trim() === text);
  if (!el) throw new Error(`no button labelled ${text}`);
  return act(async () => {
    el.dispatchEvent(new MouseEvent("click", {bubbles: true}));
  });
}

describe("AdminServiceAccounts (VATUSA/OIS#531)", () => {
  it("lists an account and flags that it holds no roles", async () => {
    const host = await mount();
    expect(host.textContent).toContain("Discord bot");
    expect(host.textContent).toContain("sa_abc123");
    // An account with no roles authenticates but can do nothing — the table must not hide that.
    expect(host.textContent).toContain("none");
  });

  // The whole point of #531: the picker has to offer BOT. The access catalog cannot, because it
  // serves ASSIGNABLE_USER_ROLES, which deliberately omits the machine roles.
  it("offers BOT in the role picker and sends it as role_names", async () => {
    put.mockResolvedValue({data: {...account, roles: ["BOT"]}, error: undefined});
    const host = await mount();
    await click(host.querySelector("tbody")!, "Roles");

    const bot = [...document.querySelectorAll("label")].find((l) =>
      l.textContent?.includes("BOT"),
    );
    expect(bot, "BOT must be offered, or the Discord bot cannot be granted its role").toBeTruthy();

    const box = bot!.querySelector("input")!;
    await act(async () => {
      box.checked = true;
      box.dispatchEvent(new MouseEvent("click", {bubbles: true}));
    });
    await click(document.body, "Save roles");

    expect(put).toHaveBeenCalledWith("/api/v1/admin/service-accounts/{id}/roles", {
      params: {path: {id: "sa-1"}},
      body: {role_names: ["BOT"]},
    });
  });

  // The plaintext token exists exactly once, in this response. If the UI fails to show it, or shows
  // it without saying so, the credential is lost and the account must be rotated to be usable.
  it("shows the new token once, with the never-again warning", async () => {
    post.mockResolvedValue({
      data: {account: {...account, id: "sa-2", name: "New bot"}, token: "ois_sa_deadbeef"},
      error: undefined,
    });
    const host = await mount();
    await click(host.querySelector('[data-testid="header-actions"]')!, "New service account");

    await type(host.querySelector("input")!, "New bot");
    await click(host, "Create account");

    expect(host.textContent).toContain("never be shown again");
    const revealed = [...host.querySelectorAll("input")].some(
      (i) => (i as HTMLInputElement).value === "ois_sa_deadbeef",
    );
    expect(revealed, "the token itself must be on screen").toBe(true);
    // No roles were picked, so no roles call should have been attempted.
    expect(put).not.toHaveBeenCalled();
    // The default lifetime is sent, not left to chance.
    expect(post.mock.calls[0][1].body.expires_in_days).toBe(90);
  });
});

describe("grants and credential lifetime (VATUSA/OIS#584)", () => {
  it("flags a stale credential and shows when it expires", async () => {
    const host = await mount([{...account, stale: true}]);
    expect(host.textContent).toContain("stale");
    expect(host.textContent).toContain(new Date("2026-11-30T00:00:00Z").toLocaleDateString());
  });

  it("lists an account's scoped grants and saves them back as (permission, artcc)", async () => {
    const granted = {...account, permissions: [{permission: "flow.fca.update", artcc_id: "ZDC"}]};
    put.mockResolvedValue({data: granted, error: undefined});
    const host = await mount([granted]);
    expect(host.textContent).toContain("flow.fca.update@ZDC");

    await click(host.querySelector("tbody")!, "Permissions");
    await click(document.body, "Save permissions");

    expect(put).toHaveBeenCalledWith("/api/v1/admin/service-accounts/{id}/permissions", {
      params: {path: {id: "sa-1"}},
      body: {permissions: [{permission: "flow.fca.update", artcc_id: "ZDC"}]},
    });
  });

  it("rotates a token with a fresh lifetime and reveals the new one", async () => {
    post.mockResolvedValue({data: {account, token: "ois_sa_rotated"}, error: undefined});
    const host = await mount();
    await click(host.querySelector("tbody")!, "Rotate");
    await click(document.body, "Rotate token");

    expect(post).toHaveBeenCalledWith("/api/v1/admin/service-accounts/{id}/rotate", {
      params: {path: {id: "sa-1"}},
      body: {expires_in_days: 90},
    });
    const revealed = [...host.querySelectorAll("input")].some(
      (i) => (i as HTMLInputElement).value === "ois_sa_rotated",
    );
    expect(revealed, "the rotated token must be shown once").toBe(true);
  });
});

/**
 * AC2's last clause — "Nothing persists it client-side" — is the one no behavioural test can see. The
 * tests above prove the token is *shown* once; none of them would notice a `localStorage.setItem`
 * beside the reveal, and adding one leaves all of them green.
 *
 * It matters more than a usual missing guard because of what the value is: a bearer credential in web
 * storage turns a one-time reveal into a permanently exfiltratable secret, readable by any future XSS
 * long after the admin closed the modal. The reveal's whole safety property is that the plaintext
 * exists only in a React state that dies with the component.
 *
 * So this reads the source, in the same spirit as `features/dashboard/nas-template-gate.test.ts` and
 * `lib/desktop-events.guard.test.ts` (#439): crude on purpose, and aimed at the one
 * thing that must not silently change.
 */
describe("the token never reaches client storage (VATUSA/OIS#531 AC2)", () => {
  const FILES = [
    "src/pages/admin/service-accounts.tsx",
    "src/lib/service-accounts.ts",
  ];

  // Matches a *write* to any persistent store. Reads are no safer here — there is nothing legitimate
  // to read back — so the pattern deliberately catches both.
  const STORAGE = /\b(localStorage|sessionStorage|indexedDB|openDatabase)\b|document\s*\.\s*cookie/;

  it.each(FILES)("%s touches no persistent storage API", async (file) => {
    const { readFile } = await import("node:fs/promises");
    const { resolve } = await import("node:path");
    const src = await readFile(resolve(process.cwd(), file), "utf8");

    const hit = STORAGE.exec(src);
    expect(
      hit?.[0],
      `${file} must not persist a service-account token client-side (AC2); found ${hit?.[0]}`,
    ).toBeUndefined();
  });
});
