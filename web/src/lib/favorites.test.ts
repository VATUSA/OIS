import {describe, expect, it} from "vitest";

import {type Favorite, isFavorite, toggleFavorite} from "./favorites";

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
