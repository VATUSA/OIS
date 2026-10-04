// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider, TooltipProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

const me = vi.hoisted(() => ({value: {server_admin: false, permissions: {} as Record<string, unknown>}}));

vi.mock("@/lib/api", () => ({ois: {GET: vi.fn(), DELETE: vi.fn()}}));
vi.mock("@/components/shell/page-meta", () => ({usePageHeader: () => {}}));
vi.mock("@/lib/auth", () => ({useMe: () => ({data: me.value})}));

import {AdminDiagnostics} from "./diagnostics";

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
});

const summary = {
  id: "r-1",
  created_at: "2026-10-03T20:00:00Z",
  user_cid: 1234567,
  user_display_name: "Ada Controller",
  user_artcc: "ZDC",
  app_version: "0.2.0",
  os: "macos",
  os_version: "15.1",
  arch: "aarch64",
  window_label: "main",
  route: "/flow",
  has_note: true,
  logs_bytes: 2048,
};

/** Mounts the page against a seeded cache — nothing reaches the network. */
async function mount(permissions: Record<string, unknown>) {
  me.value = {server_admin: false, permissions};
  const qc = new QueryClient({
    defaultOptions: {queries: {retry: false, refetchOnMount: false, staleTime: Infinity}},
  });
  qc.setQueryData(["diagnostics", 1, 50], {items: [summary], total: 1, page: 1, page_size: 50});
  qc.setQueryData(["diagnostics", "report", "r-1"], {
    ...summary,
    webview_version: "620.1",
    note: "the map went white",
    meta: {context: {webgl2: false}},
  });
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <TooltipProvider>
            <AdminDiagnostics />
          </TooltipProvider>
        </ToastProvider>
      </QueryClientProvider>,
    );
  });
  return host;
}

const buttonNamed = (label: string) =>
  [...document.querySelectorAll("button")].find((b) => b.textContent === label);

describe("AdminDiagnostics", () => {
  it("lists reports with who sent them, from where", async () => {
    const host = await mount({diagnostics: {reports: ["read"]}});
    expect(host.textContent).toContain("Ada Controller");
    expect(host.textContent).toContain("1234567");
    expect(host.textContent).toContain("/flow");
  });

  it("opens a report with its note and details, and offers delete only to holders of it", async () => {
    await mount({diagnostics: {reports: ["read"]}});
    await act(async () => {
      (document.querySelector("tbody tr") as HTMLElement).click();
    });
    expect(document.body.textContent).toContain("the map went white");
    expect(document.body.textContent).toContain('"webgl2": false');
    expect(buttonNamed("Download logs")).toBeDefined();
    expect(buttonNamed("Delete")).toBeUndefined();
  });

  it("offers delete with diagnostics.reports.delete", async () => {
    await mount({diagnostics: {reports: ["read", "delete"]}});
    await act(async () => {
      (document.querySelector("tbody tr") as HTMLElement).click();
    });
    expect(buttonNamed("Delete")).toBeDefined();
  });
});
