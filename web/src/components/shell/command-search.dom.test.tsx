// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

// The only seam that can't be driven for real: `useNavigate`/`useRouterState` need a live router.
// Everything else — who is signed in, what is favorited — goes through the real query cache and a
// stubbed `fetch`, so these assert the shipped wiring rather than a mock of it (VATUSA/OIS#312).
const here = vi.hoisted(() => ({ pathname: "/ops/tmu", href: "/ops/tmu" }));
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => () => {},
  useRouterState: ({ select }: { select: (s: unknown) => unknown }) =>
    select({ location: { pathname: here.pathname, href: here.href, search: {} }, matches: [] }),
}));

import {CommandSearch, openCommandSearch} from "./command-search";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}

beforeAll(() => {
  Element.prototype.scrollIntoView = () => {};
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

const roots: { root: ReturnType<typeof createRoot>; host: HTMLElement }[] = [];
afterEach(() => {
  for (const { root, host } of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
});

const ME = { cid: 1, server_admin: true, permissions: {} } as never;

/** Mounts CommandSearch with the palette *closed*, which is when ⌘⇧F favorites the current page. */
async function mountClosed(at: { pathname: string; href: string }, stored: unknown[] = []) {
  here.pathname = at.pathname;
  here.href = at.href;
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false, refetchInterval: false } } });
  qc.setQueryData(["me"], ME);
  // `toggle` refuses to save until the stored list has loaded, so seed it.
  qc.setQueryData(["preferences", "favorites"], { items: stored });

  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({ root, host });
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <CommandSearch />
        </ToastProvider>
      </QueryClientProvider>,
    );
  });
  // Synchronous on purpose: `toggle` writes the new list optimistically and then POSTs it, and with
  // no backend here that POST rejects and `onError` rolls the cache straight back. Awaiting the act
  // lets the rollback land first, which made this flaky — so read the write as it happens.
  const favorite = () => {
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "f", metaKey: true, shiftKey: true, bubbles: true }));
    });
    return (qc.getQueryData(["preferences", "favorites"]) as { items: { id: string; label: string }[] }).items;
  };
  return { favorite };
}

/**
 * Mounts the real CommandSearch with the palette open. Whether favorites are *fetched* is read off
 * the query cache rather than `fetch`: the generated client captures `fetch` when the module loads,
 * so a stub installed here would never be seen.
 */
async function mountSearch({ me, favorites }: { me?: unknown; favorites?: unknown[] } = {}) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false, refetchInterval: false } } });
  qc.setQueryData(["me"], me ?? null);
  if (favorites) qc.setQueryData(["preferences", "favorites"], { items: favorites });

  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({ root, host });
  await act(async () => {
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <CommandSearch />
        </ToastProvider>
      </QueryClientProvider>,
    );
  });
  await act(async () => void openCommandSearch());
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  return {
    // Whether the query is *enabled*, not whether its request has settled. Reading the settled status
    // after one tick only passed when nothing listened on API_BASE — the refused request errored in
    // time — and failed against a live backend, whose round trip was still in flight (VATUSA/OIS#374).
    favoritesEnabled: () =>
      qc.getQueryCache().find({ queryKey: ["preferences", "favorites"] })?.isDisabled() === false,
    stars: () => document.querySelectorAll("[aria-pressed]"),
    groupHeadings: () =>
      Array.from(document.querySelectorAll("[data-index]"))
        .length === 0
        ? []
        : Array.from(document.querySelectorAll("div.uppercase")).map((n) => n.textContent),
  };
}

describe("command search, signed out (VATUSA/OIS#312)", () => {
  // M16: without `onToggleStar` no row renders a star, so a signed-out visitor is never offered one.
  it("offers no favorite star on any row", async () => {
    const p = await mountSearch();
    expect(p.stars()).toHaveLength(0);
  });

  // M17: favorites are per user — there is nothing to fetch and nothing that could be saved.
  it("never requests the favorites preferences", async () => {
    const p = await mountSearch();
    expect(p.favoritesEnabled()).toBe(false);
  });
});

describe("command search, signed in (VATUSA/OIS#312)", () => {
  it("does request the favorites preferences", async () => {
    const p = await mountSearch({ me: ME });
    expect(p.favoritesEnabled()).toBe(true);
  });

  it("offers a star once there is a user to favorite for", async () => {
    const p = await mountSearch({ me: ME });
    expect(p.stars().length).toBeGreaterThan(0);
  });

  // M7: favorites are pinned — the group has to come first, not merely exist.
  it("pins the Favorites group above every other group", async () => {
    const p = await mountSearch({
      me: ME,
      favorites: [{ kind: "page", id: "/ops/tmu", label: "TMU", href: "/ops/tmu" }],
    });
    expect(p.groupHeadings()[0]).toBe("Favorites");
  });
});

describe("⌘⇧F on the current page (VATUSA/OIS#312)", () => {
  // Finding 1: this used to read `document.title`, which nothing in the app ever assigns — so every
  // page below a nav item was stored as "OIS", and two events read identically.
  it("labels the favorite from the page's own title, never the document title", async () => {
    document.title = "OIS";
    const p = await mountClosed({ pathname: "/ops/tmu", href: "/ops/tmu" });
    const stored = p.favorite();
    expect(stored[0].label).not.toBe("OIS");
    expect(stored[0].label).toBe("TMU");
  });

  // Finding 6: several routes carry their identity in `search`, so keying on the path alone made
  // two airports one favorite that overwrote itself.
  it("keys the favorite on the full href, so two airports are two favorites", async () => {
    const p = await mountClosed(
      { pathname: "/ops/airport", href: "/ops/airport?icao=KSFO" },
      [{ kind: "page", id: "/ops/airport?icao=KDEN", label: "Airport", href: "/ops/airport?icao=KDEN" }],
    );
    const stored = p.favorite();
    expect(stored.map((f) => f.id).sort()).toEqual([
      "/ops/airport?icao=KDEN",
      "/ops/airport?icao=KSFO",
    ]);
  });
});
