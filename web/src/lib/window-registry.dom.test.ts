// @vitest-environment jsdom
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";

import {forgetWindow, rememberWindow, rememberedWindows} from "./window-registry";

/** This project's jsdom provides no localStorage, so the tests supply one. */
function installStorage() {
  const store = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
    removeItem: (k: string) => void store.delete(k),
    clear: () => store.clear(),
  });
  return store;
}

let store: Map<string, string>;

beforeEach(() => {
  store = installStorage();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

const IDST = {id: "/ops/idst", route: "/ops/idst", title: "OIS · IDST"};
const MAP = {id: "/facility-map", route: "/facility-map", title: "OIS · Map"};

describe("the remembered window set", () => {
  it("starts empty", () => {
    expect(rememberedWindows()).toEqual([]);
  });

  it("remembers each window that was opened", () => {
    rememberWindow(IDST);
    rememberWindow(MAP);
    expect(rememberedWindows()).toEqual([IDST, MAP]);
  });

  it("replaces rather than duplicates when the same window is reopened", () => {
    rememberWindow(IDST);
    rememberWindow({...IDST, title: "OIS · IDST (renamed)"});

    const windows = rememberedWindows();
    expect(windows).toHaveLength(1);
    expect(windows[0]!.title).toBe("OIS · IDST (renamed)");
  });

  it("forgets a window the user closed, so it stays closed next launch", () => {
    rememberWindow(IDST);
    rememberWindow(MAP);

    forgetWindow(IDST.id);

    expect(rememberedWindows()).toEqual([MAP]);
  });

  it("forgetting one that isn't there is harmless", () => {
    rememberWindow(IDST);
    forgetWindow("/never-opened");
    expect(rememberedWindows()).toEqual([IDST]);
  });

  it("keeps the good entries when one stored entry is malformed", () => {
    // A single bad entry from an older version must not cost the user every other window.
    store.set("ois.windows", JSON.stringify([IDST, {id: 5}, null, MAP]));
    expect(rememberedWindows()).toEqual([IDST, MAP]);
  });

  it("recovers from storage that isn't JSON at all", () => {
    store.set("ois.windows", "}{");
    expect(rememberedWindows()).toEqual([]);
  });

  it("recovers when storage holds JSON that isn't a list", () => {
    store.set("ois.windows", JSON.stringify({id: "x"}));
    expect(rememberedWindows()).toEqual([]);
  });
});
