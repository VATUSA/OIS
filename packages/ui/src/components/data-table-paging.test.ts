import {describe, expect, it} from "vitest";

import {pageWindow, toggleAll, toggleId} from "./data-table-paging";

const opts = { rowCap: 10, expanded: false, page: 1, pageSize: 50 };

describe("pageWindow", () => {
  it("shows everything with no controls when under the cap", () => {
    expect(pageWindow(7, opts)).toMatchObject({ start: 0, end: 7, canExpand: false, canCollapse: false, pageCount: 1 });
  });

  it("caps a longer list and offers expand", () => {
    expect(pageWindow(120, opts)).toEqual({ start: 0, end: 10, canExpand: true, canCollapse: false, pageCount: 1, page: 1 });
  });

  it("offers no Show-all when the list is exactly the cap", () => {
    expect(pageWindow(10, opts)).toEqual({ start: 0, end: 10, canExpand: false, canCollapse: false, pageCount: 1, page: 1 });
    expect(pageWindow(11, opts)).toMatchObject({ end: 10, canExpand: true });
  });

  it("paginates once expanded, and offers collapse", () => {
    expect(pageWindow(120, { ...opts, expanded: true })).toMatchObject({
      start: 0, end: 50, canExpand: false, canCollapse: true, pageCount: 3, page: 1,
    });
    expect(pageWindow(120, { ...opts, expanded: true, page: 3 })).toMatchObject({ start: 100, end: 120, page: 3 });
  });

  it("pages straight away with no Show-all step when the cap is Infinity", () => {
    expect(pageWindow(120, { ...opts, rowCap: Infinity, pageSize: 25, page: 2 })).toMatchObject({
      start: 25, end: 50, canExpand: false, canCollapse: false, pageCount: 5, page: 2,
    });
  });

  it("clamps an out-of-range page (data shrank)", () => {
    expect(pageWindow(30, { ...opts, rowCap: 5, expanded: true, page: 9, pageSize: 10 })).toMatchObject({
      page: 3, start: 20, end: 30,
    });
  });

  it("clamps a page below 1 to the first page", () => {
    expect(pageWindow(30, { ...opts, rowCap: Infinity, page: 0, pageSize: 10 })).toMatchObject({ page: 1, start: 0, end: 10 });
    expect(pageWindow(30, { ...opts, rowCap: Infinity, page: -2, pageSize: 10 })).toMatchObject({ page: 1, start: 0, end: 10 });
  });

  it("handles an empty list", () => {
    expect(pageWindow(0, opts)).toMatchObject({ start: 0, end: 0, pageCount: 1, canExpand: false });
  });
});

describe("selection helpers", () => {
  it("toggles one id", () => {
    expect([...toggleId(new Set(["a"]), "b")]).toEqual(["a", "b"]);
    expect([...toggleId(new Set(["a"]), "a")]).toEqual([]);
  });

  it("select-all selects the rest, then clears", () => {
    const some = toggleAll(new Set(["a"]), ["a", "b"]);
    expect([...some].sort()).toEqual(["a", "b"]);
    expect([...toggleAll(some, ["a", "b"])]).toEqual([]);
  });
});
