import {describe, expect, it, vi} from "vitest";

import type {Me} from "./auth";
import {
  type Favorite,
  canSeeFavorite,
  favoriteHref,
  favoritePageLabel,
  favoriteRow,
  isFavorite,
  toggleFavorite,
  unavailable,
} from "./favorites";

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

describe("favoritePageLabel", () => {
  // The title the shell shows wins, so a favorite can never be named something else on screen.
  it("uses the page's own title, so detail pages stay distinct", () => {
    expect(favoritePageLabel("/admin/planning/events/4821", "Cross the Pond")).toBe("Cross the Pond");
    expect(favoritePageLabel("/admin/planning/events/9137", "Light the Night")).toBe("Light the Night");
    expect(favoritePageLabel("/facility-map/ZDV", "ZDV — Denver Center")).toBe("ZDV — Denver Center");
  });

  it("falls back to the nav item only when the page declares no title", () => {
    expect(favoritePageLabel("/ops/tmu", undefined)).toBe("TMU");
  });

  it("falls back to the path when there is no title and no nav item either", () => {
    expect(favoritePageLabel("/nowhere", undefined)).toBe("/nowhere");
  });

  // The old fallback was `document.title`, which nothing in this app ever sets — every detail page
  // was stored as the literal "OIS" from index.html.
  it("never names a page after the static document title", () => {
    expect(favoritePageLabel("/admin/planning/events/4821", undefined)).not.toBe("OIS");
  });
});

describe("favoriteRow", () => {
  const item = { id: "tmi:t1", label: "20MIT", onSelect: () => {} };
  const fav: Favorite = { kind: "tmi", id: "t1", label: "20MIT", href: "/ops/tmu" };
  const store = (starred: boolean) => ({ isFavorite: () => starred, toggle: vi.fn(() => true) });

  it("offers no star at all to a signed-out visitor", () => {
    const favorites = store(false);
    const row = favoriteRow(null, favorites, item, fav);
    expect(row).toEqual(item);
    expect(row.onToggleStar).toBeUndefined();
    expect(row.starred).toBeUndefined();
  });

  it("stars a row for a signed-in user, reflecting whether it is already a favorite", () => {
    expect(favoriteRow(holding(), store(false), item, fav).starred).toBe(false);
    expect(favoriteRow(holding(), store(true), item, fav).starred).toBe(true);
  });

  it("toggles this row's favorite, and pairs the row with its pinned twin", () => {
    const favorites = store(false);
    const row = favoriteRow(holding(), favorites, item, fav);
    row.onToggleStar!();
    expect(favorites.toggle).toHaveBeenCalledWith(fav);
    // The pinned Favorites row is built from the same favorite, so both carry the same entity.
    expect(row.entity).toBe("tmi:t1");
    expect(favoriteRow(holding(), favorites, { ...item, id: "favorite:tmi:t1" }, fav).entity).toBe(row.entity);
  });
});
