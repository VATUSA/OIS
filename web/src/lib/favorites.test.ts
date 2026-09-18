import {describe, expect, it} from "vitest";

import type {Me} from "./auth";
import {
  type Favorite,
  type FavoriteScopes,
  canSeeFavorite,
  favoriteHref,
  favoriteUnavailable,
  isFavorite,
  toggleFavorite,
  withPinnedFavorites,
} from "./favorites";

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

/** A `Me` holding exactly `names` (dotted `segments.action`), same shape as nav.test.ts builds. */
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
  return { id: "u1", cid: 1, server_admin: false, role_names: [], permissions: tree } as unknown as Me;
}

const ALL_SCOPES: FavoriteScopes = { tmis: true, events: true, dashboards: true };
const fav = (over: Partial<Favorite>): Favorite => ({ kind: "page", id: "x", label: "X", href: "/", ...over });

describe("canSeeFavorite", () => {
  it("hides a kind the viewer can no longer read, and keeps the ones they can", () => {
    const scopes: FavoriteScopes = { tmis: false, events: true, dashboards: true };
    expect(canSeeFavorite(holding("tmu.tmi.read"), tmi, scopes)).toBe(false);
    expect(canSeeFavorite(holding("events.plan.read"), fav({ kind: "event", href: "/admin/planning/events/1" }), scopes)).toBe(
      true,
    );
  });

  it("gates an Admin destination on that page's own permission", () => {
    const f = fav({ kind: "airport", href: "/admin/planning/airport-configs?icao=KDEN" });
    expect(canSeeFavorite(holding("events.plan.read"), f, ALL_SCOPES)).toBe(true);
    // Holds an Admin-area link, but not this page's.
    expect(canSeeFavorite(holding("audit.logs.read"), f, ALL_SCOPES)).toBe(false);
    // No Admin-area link at all.
    expect(canSeeFavorite(holding("auth.profile.read"), f, ALL_SCOPES)).toBe(false);
  });

  it("gates a non-Admin page on its nav link's permission", () => {
    const idst = fav({ id: "/ops/idst", href: "/ops/idst" });
    expect(canSeeFavorite(holding("flow.fca.read"), idst, ALL_SCOPES)).toBe(true);
    expect(canSeeFavorite(holding("tmu.program.read"), idst, ALL_SCOPES)).toBe(false);
  });

  it("keeps a public destination for a signed-out viewer", () => {
    expect(canSeeFavorite(null, fav({ id: "/advisories", href: "/advisories" }), ALL_SCOPES)).toBe(true);
  });

  it("ignores the query string when resolving the destination's permission", () => {
    const f = fav({ kind: "tmi", id: "t9", href: "/ops/tmu?tab=restrictions&facility=ZDV" });
    expect(canSeeFavorite(holding("tmu.tmi.read"), f, ALL_SCOPES)).toBe(true);
    expect(canSeeFavorite(holding("auth.profile.read"), f, ALL_SCOPES)).toBe(false);
  });
});

describe("favoriteUnavailable", () => {
  it("is false while the source hasn't loaded, so a favorite never flashes Unavailable", () => {
    expect(favoriteUnavailable(fav({ kind: "aircraft", id: "DAL123" }), {})).toBe(false);
  });

  it("is true once the source has loaded without it", () => {
    const ac = fav({ kind: "aircraft", id: "DAL123" });
    expect(favoriteUnavailable(ac, { aircraft: [{ callsign: "UAL1" }] })).toBe(true);
    expect(favoriteUnavailable(ac, { aircraft: [{ callsign: "DAL123" }] })).toBe(false);
  });

  it("matches each kind against its own source", () => {
    expect(favoriteUnavailable(fav({ kind: "tmi", id: "t1" }), { tmis: [{ id: "t1" }] })).toBe(false);
    expect(favoriteUnavailable(fav({ kind: "event", id: "42" }), { events: [{ id: 42 }] })).toBe(false);
    expect(favoriteUnavailable(fav({ kind: "event", id: "42" }), { events: [{ id: 7 }] })).toBe(true);
    expect(favoriteUnavailable(fav({ kind: "dashboard", id: "b1" }), { dashboards: [] })).toBe(true);
  });

  it("never marks a page or airport favorite unavailable — they have no live source", () => {
    expect(favoriteUnavailable(page, { aircraft: [], tmis: [], events: [], dashboards: [] })).toBe(false);
    expect(favoriteUnavailable(fav({ kind: "airport", id: "/ops/airport:KDEN" }), { aircraft: [] })).toBe(false);
  });
});

describe("withPinnedFavorites", () => {
  const favs = { label: "Favorites" };
  const rest = [{ label: "Pages" }, { label: "Flights" }];

  it("pins Favorites above every scoped group", () => {
    expect(withPinnedFavorites(true, favs, rest).map((g) => g.label)).toEqual(["Favorites", "Pages", "Flights"]);
  });

  it("offers no Favorites group to a signed-out viewer", () => {
    expect(withPinnedFavorites(false, favs, rest).map((g) => g.label)).toEqual(["Pages", "Flights"]);
  });

  it("does not mutate the scoped groups it was given", () => {
    const scoped = [{ label: "Pages" }];
    withPinnedFavorites(true, favs, scoped);
    expect(scoped).toEqual([{ label: "Pages" }]);
  });
});
