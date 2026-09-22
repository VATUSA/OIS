// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider, defaultScheduler, notifyManager} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

// The only seam that can't be driven for real: `useNavigate`/`useRouterState` need a live router.
// Everything else — who is signed in, what is favorited — goes through the real query cache and a
// stubbed `fetch`, so these assert the shipped wiring rather than a mock of it (VATUSA/OIS#312).
const here = vi.hoisted(() => ({
  pathname: "/ops/tmu",
  href: "/ops/tmu",
  // The matched routes' `staticData`, which is where a route declares its own title.
  matches: [] as { staticData: { title?: string } }[],
}));
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => () => {},
  useRouterState: ({ select }: { select: (s: unknown) => unknown }) =>
    select({ location: { pathname: here.pathname, href: here.href, search: {} }, matches: here.matches }),
}));

import {CommandSearch, openCommandSearch} from "./command-search";
import {PageMetaProvider, usePageHeader} from "./page-meta";

/** A page that names itself at runtime, as an event's detail page does with the event's name. */
function PageTitled({ title }: { title: string }) {
  usePageHeader({ title });
  return null;
}

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
  here.matches = [];
  vi.unstubAllGlobals();
});

const ME = { cid: 1, server_admin: true, permissions: {} } as never;

/** Mounts CommandSearch with the palette *closed*, which is when ⌘⇧F favorites the current page. */
async function mountClosed(
  at: { pathname: string; href: string },
  stored: unknown[] = [],
  // The title the page itself sets at runtime; mounted inside `PageMetaProvider`, as `app-shell` does.
  pageTitle?: string,
) {
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
          <PageMetaProvider>
            {pageTitle && <PageTitled title={pageTitle} />}
            <CommandSearch />
          </PageMetaProvider>
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
    return (qc.getQueryData(["preferences", "favorites"]) as { items: { id: string; label: string; href: string }[] }).items;
  };
  return { favorite };
}

/** A keydown on the open palette's search field, which owns every palette shortcut. */
const paletteKey = (init: KeyboardEventInit) =>
  act(() => void document.querySelector("input")!.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, ...init })));
/** Types into the open palette's search field, through React's own value tracking. */
const paletteType = (text: string) =>
  act(() => {
    const input = document.querySelector("input")!;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, text);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
/** The highlighted palette row and the group it sits in. */
const highlightedRow = () => {
  const row = Array.from(document.querySelectorAll("[data-index]")).find((r) => r.className.includes("bg-panel-2"));
  // Each group is a wrapper whose first child is its heading.
  return { text: row?.textContent, group: row?.closest(".mb-1")?.firstElementChild?.textContent };
};

/**
 * Mounts the real CommandSearch with the palette open. Whether favorites are *fetched* is read off
 * the query cache rather than `fetch`: the generated client captures `fetch` when the module loads,
 * so a stub installed here would never be seen — and a disabled query is exactly one that sits at
 * fetchStatus "idle".
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
    // A query that never ran sits at "pending" forever; one that ran resolves or errors. `fetchStatus`
    // is no good here — it is back to "idle" either way once the request has settled.
    favoritesRan: () => (qc.getQueryState(["preferences", "favorites"])?.status ?? "pending") !== "pending",
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
    expect(p.favoritesRan()).toBe(false);
  });
});

describe("command search, signed in (VATUSA/OIS#312)", () => {
  it("does request the favorites preferences", async () => {
    const p = await mountSearch({ me: ME });
    expect(p.favoritesRan()).toBe(true);
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

describe("⌘⇧F on a page below a nav item (VATUSA/OIS#339)", () => {
  // Both tests above favorite nav items themselves, which resolve through the nav fallback whether or
  // not the page's own title wins — so they could not catch every event being stored as "Events".
  it("labels the favorite with the title the page sets for itself", async () => {
    const p = await mountClosed(
      { pathname: "/admin/planning/events/4821", href: "/admin/planning/events/4821" },
      [],
      "Cactus Crossing",
    );
    expect(p.favorite()[0].label).toBe("Cactus Crossing");
  });

  it("labels the favorite with its route's title when the page sets none", async () => {
    here.matches = [{ staticData: { title: "Event builder" } }];
    const p = await mountClosed({ pathname: "/admin/planning/events/9137", href: "/admin/planning/events/9137" });
    expect(p.favorite()[0].label).toBe("Event builder");
  });
});

describe("⌘⇧F across a page's view switch (VATUSA/OIS#339)", () => {
  it("keys the favorite on the page, not the view it was on", async () => {
    const p = await mountClosed({ pathname: "/ops/tmu", href: "/ops/tmu?view=board" });
    const [stored] = p.favorite();
    expect(stored.id).toBe("/ops/tmu");
    expect(stored.href).toBe("/ops/tmu");
  });

  it("un-stars one stored before the key dropped ?view=, rather than adding a second", async () => {
    // Favorites shipped on main, so this is what the users who already hit #339 actually have
    // saved. Without normalising the stored list, the new key misses it, `toggleFavorite` adds,
    // and they get two identical "TMU" rows with the old one unreachable from the keyboard.
    const p = await mountClosed({ pathname: "/ops/tmu", href: "/ops/tmu?view=board" }, [
      { kind: "page", id: "/ops/tmu?view=board", label: "TMU", href: "/ops/tmu?view=board" },
    ]);
    expect(p.favorite()).toEqual([]);
  });

  it("treats a second view of a favorited page as the same favorite, not a duplicate", async () => {
    // Stored exactly as the palette's own Pages row stores it (`id: p.to`).
    const p = await mountClosed({ pathname: "/ops/tmu", href: "/ops/tmu?view=table" }, [
      { kind: "page", id: "/ops/tmu", label: "TMU", href: "/ops/tmu" },
    ]);
    expect(p.favorite()).toEqual([]);
  });
});

describe("un-starring a pinned favorite in the palette (VATUSA/OIS#339)", () => {
  // The pinned row and the row it was starred from share `entityId`; that is what lets the highlight
  // land on the source row instead of holding a position that now belongs to something else.
  it("moves the highlight to the row the favorite was starred from", async () => {
    await mountSearch({ me: ME, favorites: [{ kind: "page", id: "/ops/tmu", label: "TMU", href: "/ops/tmu" }] });
    // The Pages scope lists every page with Home first, so the source row sits well below where the
    // pinned one was — holding position would land on a different page, and only `entityId` finds it.
    paletteType("@pages ");
    paletteKey({ key: "ArrowDown" });
    paletteKey({ key: "ArrowUp" });
    expect(highlightedRow()).toMatchObject({ text: expect.stringContaining("TMU"), group: "Favorites" });
    // Synchronous, like `favorite` above: read the optimistic write before the failed save rolls it
    // back. The cache only reaches the palette through `notifyManager`, which defers to a macrotask,
    // so deliver inline for this one press or the re-render would wait on the rollback's race.
    notifyManager.setScheduler((cb) => cb());
    try {
      paletteKey({ key: "f", metaKey: true, shiftKey: true });
    } finally {
      notifyManager.setScheduler(defaultScheduler);
    }
    expect(highlightedRow()).toMatchObject({ text: expect.stringContaining("TMU"), group: "Pages" });
  });
});
