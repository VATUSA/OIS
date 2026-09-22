import {describe, expect, it} from "vitest";

import type {Me} from "./auth";
import { canSeeFavorite, favoriteHref, isFavorite, isFavoriteHotkey, normalizeFavorites, pageFavoriteHref, pageFavoriteHrefFor, toggleFavorite, type Favorite, unavailable } from "./favorites";

/** A permission tree holding exactly `names` (dotted `segments.action`), as in `nav.test.ts`. */
function holding(...names: string[]): Me {
  const tree: Record<string, unknown> = {};
  for (const name of names) {
    const parts = name.split(".");
    const action = parts.pop()!;
    let node = tree;
    for (const [i, seg] of parts.entries()) {
      if (i === parts.length - 1) node[seg] = [...((node[seg] as string[]) ?? []), action];
      else node = (node[seg] ??= {}) as Record<string, unknown>;
    }
  }
  return {
    id: "u1",
    cid: 1,
    email: "a@b.c",
    display_name: "Tester",
    rating: null,
    server_admin: false,
    role_names: [],
    permissions: tree as Me["permissions"],
  } as Me;
}

const tmi: Favorite = { kind: "tmi", id: "t1", label: "KJFK 20MIT", href: "/ops/tmu?tab=restrictions" };
const page: Favorite = { kind: "page", id: "/ops/tmu", label: "TMU", href: "/ops/tmu" };

describe("toggleFavorite", () => {
  it("adds a new favorite to the front", () => {
    expect(toggleFavorite([page], tmi)).toEqual([tmi, page]);
  });

  it("removes an existing favorite by kind and id, ignoring a stale label", () => {
    expect(toggleFavorite([tmi, page], { ...tmi, label: "renamed" })).toEqual([page]);
  });

  it("does not confuse the same id across kinds", () => {
    const sameIdOtherKind: Favorite = { ...tmi, kind: "event" };
    expect(toggleFavorite([tmi], sameIdOtherKind)).toEqual([sameIdOtherKind, tmi]);
  });
});

describe("isFavorite", () => {
  it("matches kind and id", () => {
    expect(isFavorite([tmi], "tmi", "t1")).toBe(true);
    expect(isFavorite([tmi], "event", "t1")).toBe(false);
    expect(isFavorite([], "tmi", "t1")).toBe(false);
  });
});

describe("favoriteHref", () => {
  it("carries the row's search params, so the favorite reopens what the row opens", () => {
    expect(favoriteHref({ to: "/admin/planning/airport-configs", search: { icao: "KDEN" } })).toBe(
      "/admin/planning/airport-configs?icao=KDEN",
    );
    expect(favoriteHref({ to: "/ops/tmu", search: { tab: "restrictions", facility: "ZDV" } })).toBe(
      "/ops/tmu?tab=restrictions&facility=ZDV",
    );
  });

  it("escapes a param that isn't URL-safe", () => {
    expect(favoriteHref({ to: "/advisories/fcas", search: { flight: "N1 2A" } })).toBe("/advisories/fcas?flight=N1+2A");
  });
});

describe("pageFavoriteHref (VATUSA/OIS#339)", () => {
  it("drops the view switch, so every view of a page is one favorite — the one its nav row stars", () => {
    expect(pageFavoriteHref("/ops/tmu?view=board")).toBe("/ops/tmu");
    expect(pageFavoriteHref("/ops/tmu?view=table")).toBe("/ops/tmu");
  });

  it("keeps the params that say what the page is about", () => {
    expect(pageFavoriteHref("/ops/airport?icao=KSFO&view=table")).toBe("/ops/airport?icao=KSFO");
  });

  it("leaves a relative href with no view as it is", () => {
    for (const href of ["/ops/airport?icao=KSFO", "/advisories/fcas?flight=N1+2A", "/ops/tmu"]) {
      expect(pageFavoriteHref(href)).toBe(href);
    }
  });

  // The router's href is absolute; the Pages row keys on a relative `to`. One key for both (#339 review).
  it("makes an absolute href relative, so it matches the page's own row", () => {
    expect(pageFavoriteHref("http://localhost:5173/dashboard")).toBe("/dashboard");
    expect(pageFavoriteHref("https://ois.vatusa.net/ops/airport?icao=KDCA")).toBe("/ops/airport?icao=KDCA");
  });
});

describe("pageFavoriteHrefFor (VATUSA/OIS#339 review)", () => {
  it("builds the key from the validated search, dropping empty values and the view", () => {
    expect(pageFavoriteHrefFor("/ops/tmu", { facility: "ZDV", tab: undefined, view: "board" })).toBe(
      "/ops/tmu?facility=ZDV",
    );
    expect(pageFavoriteHrefFor("/dashboard", undefined)).toBe("/dashboard");
  });
});

describe("canSeeFavorite", () => {
  const tmiFav: Favorite = { kind: "tmi", id: "t1", label: "20MIT", href: "/ops/tmu?tab=restrictions&facility=ZDV" };
  const eventFav: Favorite = { kind: "event", id: "9", label: "Cross the Pond", href: "/admin/planning/events/9" };
  const fcaPage: Favorite = { kind: "page", id: "/advisories/fcas", label: "FCAs", href: "/advisories/fcas" };

  it("drops a favorite whose kind permission is gone", () => {
    expect(canSeeFavorite(holding("tmu.tmi.read", "tmu.program.read"), tmiFav)).toBe(true);
    expect(canSeeFavorite(holding("tmu.program.read"), tmiFav)).toBe(false);
  });

  it("drops an admin favorite whose destination is no longer reachable", () => {
    expect(canSeeFavorite(holding("events.plan.read"), eventFav)).toBe(true);
    expect(canSeeFavorite(holding("tmu.tmi.read"), eventFav)).toBe(false);
  });

  it("keeps a public page for a user holding nothing, and drops everything for a signed-out visitor", () => {
    expect(canSeeFavorite(holding(), fcaPage)).toBe(true);
    expect(canSeeFavorite(null, tmiFav)).toBe(false);
    expect(canSeeFavorite(null, eventFav)).toBe(false);
  });

  it("ignores the query string when gating the destination", () => {
    const withQuery: Favorite = { ...fcaPage, href: "/advisories/fcas?flight=UAL1" };
    expect(canSeeFavorite(holding(), withQuery)).toBe(true);
  });
});

describe("unavailable", () => {
  const tmiFav: Favorite = { kind: "tmi", id: "t1", label: "20MIT", href: "/ops/tmu" };

  it("is false while the source hasn't loaded, so nothing is wrongly marked gone", () => {
    expect(unavailable(tmiFav, {})).toBe(false);
  });

  it("is true once the loaded source doesn't hold it", () => {
    expect(unavailable(tmiFav, { tmis: [{ id: "other" }] })).toBe(true);
    expect(unavailable(tmiFav, { tmis: [] })).toBe(true);
  });

  it("is false while the entity is still there", () => {
    expect(unavailable(tmiFav, { tmis: [{ id: "t1" }] })).toBe(false);
  });

  it("matches each kind against its own source", () => {
    const flight: Favorite = { kind: "aircraft", id: "UAL1", label: "UAL1", href: "/advisories/fcas" };
    const event: Favorite = { kind: "event", id: "9", label: "CTP", href: "/admin/planning/events/9" };
    const board: Favorite = { kind: "dashboard", id: "b1", label: "Board", href: "/ops/my/b1" };
    expect(unavailable(flight, { aircraft: [{ callsign: "UAL1" }] })).toBe(false);
    expect(unavailable(flight, { aircraft: [{ callsign: "DAL2" }] })).toBe(true);
    expect(unavailable(event, { events: [{ id: 9 }] })).toBe(false);
    expect(unavailable(board, { dashboards: [{ id: "other" }] })).toBe(true);
  });

  it("never marks a page or airport gone — they have no source", () => {
    const page: Favorite = { kind: "page", id: "/ops/tmu", label: "TMU", href: "/ops/tmu" };
    const airport: Favorite = { kind: "airport", id: "/ops/airport:KDEN", label: "KDEN", href: "/ops/airport?icao=KDEN" };
    expect(unavailable(page, { tmis: [] })).toBe(false);
    expect(unavailable(airport, { tmis: [] })).toBe(false);
  });
});

describe("isFavoriteHotkey", () => {
  const key = (over: Partial<KeyboardEvent>) =>
    ({ metaKey: false, ctrlKey: false, shiftKey: false, key: "f", ...over }) as KeyboardEvent;

  it("matches ⌘⇧F and Ctrl+⇧F", () => {
    expect(isFavoriteHotkey(key({ metaKey: true, shiftKey: true }))).toBe(true);
    expect(isFavoriteHotkey(key({ ctrlKey: true, shiftKey: true }))).toBe(true);
    expect(isFavoriteHotkey(key({ metaKey: true, shiftKey: true, key: "F" }))).toBe(true);
  });

  it("leaves browser find (⌘F) and find-next (⌘G) alone", () => {
    expect(isFavoriteHotkey(key({ metaKey: true }))).toBe(false);
    expect(isFavoriteHotkey(key({ ctrlKey: true }))).toBe(false);
    expect(isFavoriteHotkey(key({ metaKey: true, shiftKey: true, key: "g" }))).toBe(false);
  });

  it("ignores a bare ⇧F and a bare f", () => {
    expect(isFavoriteHotkey(key({ shiftKey: true }))).toBe(false);
    expect(isFavoriteHotkey(key({}))).toBe(false);
  });
});

describe("favorites stored before the page key dropped ?view=", () => {
  const legacy = (id: string) => ({ kind: "page" as const, id, label: "My dashboards", href: id });

  it("lets the hotkey un-star one instead of adding a duplicate", () => {
    // The users this issue was filed for already have `page:/ops/my?view=list` saved. Without
    // normalisation the new key misses it, `toggleFavorite` adds, and they end up with two
    // identical rows and no way to remove the old one from the keyboard.
    const stored = normalizeFavorites([legacy("/ops/my?view=list")]);
    const href = pageFavoriteHref("http://localhost:5173/ops/my?view=list");

    const next = toggleFavorite(stored, { kind: "page", id: href, label: "My dashboards", href });

    expect(next).toEqual([]);
  });

  it("collapses a legacy row and its new-key twin into one", () => {
    // Someone who favorited the page again after the change has both keys stored.
    const normalized = normalizeFavorites([legacy("/ops/my"), legacy("/ops/my?view=list")]);

    expect(normalized.map((f) => f.id)).toEqual(["/ops/my"]);
  });

  it("rewrites href as well as id, so the row still navigates", () => {
    const [favorite] = normalizeFavorites([legacy("/ops/tmu?view=board")]);

    expect(favorite).toMatchObject({ id: "/ops/tmu", href: "/ops/tmu" });
  });

  it("leaves non-page favorites and identity-bearing params alone", () => {
    // `?icao=` names the subject; only presentational params may be dropped.
    const items = [
      { kind: "tmi" as const, id: "t-1", label: "20 MIT", href: "/ops/tmu" },
      legacy("/ops/airport?icao=KDCA"),
    ];

    expect(normalizeFavorites(items)).toEqual(items);
  });

  it("keeps the order the user left them in", () => {
    const items = [legacy("/a"), legacy("/b?view=x"), legacy("/c")];

    expect(normalizeFavorites(items).map((f) => f.id)).toEqual(["/a", "/b", "/c"]);
  });
});
