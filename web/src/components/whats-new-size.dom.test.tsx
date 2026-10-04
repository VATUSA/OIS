// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {describe, expect, it, vi} from "vitest";

import type {ChangelogEntry} from "@/lib/changelog";

/**
 * The panel widens only when something it shows has shots (#665). The shipped changelog has none yet,
 * so this file swaps in one that does, to prove `panelSize` actually reaches the `Modal`.
 */
const entries = vi.hoisted(() => ({current: [] as ChangelogEntry[]}));
vi.mock("@/lib/changelog", async (importActual) => ({
  ...(await importActual<typeof import("@/lib/changelog")>()),
  get CHANGELOG() {
    return entries.current;
  },
}));

import {openWhatsNew, WhatsNew} from "./whats-new";

(globalThis as {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;

const entry = (id: string, withShot: boolean): ChangelogEntry => ({
  id,
  date: "2026-10-01",
  title: id,
  sections: [{highlights: ["x"], shots: withShot ? [{src: "/s.png", alt: "A shot"}] : undefined}],
});

function panelWidth(changelog: ChangelogEntry[]): string {
  entries.current = changelog;
  const qc = new QueryClient({defaultOptions: {queries: {retry: false, staleTime: Infinity}}});
  qc.setQueryData(["me"], {id: "u1", display_name: "Test", cid: 1, permissions: []});
  qc.setQueryData(["preferences", "changelog"], {lastSeenId: changelog[0]!.id});
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  act(() => root.render(<QueryClientProvider client={qc}><WhatsNew /></QueryClientProvider>));
  act(() => openWhatsNew());
  const html = document.querySelector("[role=dialog]")!.innerHTML;
  act(() => root.unmount());
  host.remove();
  return html.includes("max-w-4xl") ? "xl" : html.includes("max-w-lg") ? "md" : "?";
}

describe("the panel's width (#665)", () => {
  it("stays at its text width when nothing shown has shots", () => {
    expect(panelWidth([entry("b", false), entry("a", false)])).toBe("md");
  });

  it("widens when an entry it shows has shots", () => {
    expect(panelWidth([entry("b", false), entry("a", true)])).toBe("xl");
  });
});
