import {describe, expect, it} from "vitest";

import type {SectorDemandRow} from "./sector-demand";
import {
  colourOf,
  cmpSector,
  consolidateAllPatch,
  consolidationError,
  consolidationOf,
  deconsolidateAllPatch,
  isAlerting,
  mapText,
  menuLists,
  moveInOrder,
  orderRows,
  storageKey,
  withPending,
  zHHMM,
} from "./view";

const row = (sector_id: string, consolidated: string[] = [], levels: ("ok" | "watch" | "over")[] = []): SectorDemandRow => ({
  sector_id,
  name: null,
  tier: "high",
  limit: 10,
  limit_overridden: false,
  consolidated,
  bins: levels.map((level) => ({active: 0, proposed: 0, combined: 0, level})),
});

describe("the vTBFM board's formatting (#794)", () => {
  it("names a level's colour without judging it", () => {
    expect(["ok", "watch", "over"].map((l) => colourOf(l as "ok"))).toEqual(["green", "yellow", "red"]);
  });

  it("formats MAP and bin labels like vTBFM", () => {
    expect([mapText(10), mapText(2), mapText(123)]).toEqual(["10/10", "02/02", "123/123"]);
    expect(zHHMM(Date.UTC(2026, 9, 7, 4, 15))).toBe("0415");
    expect(zHHMM(Date.UTC(2026, 9, 7, 0, 0))).toBe("0000");
  });

  it("orders sectors numerically when both are numbers, else lexically", () => {
    expect(["100", "16", "9", "A80", "025"].sort(cmpSector)).toEqual(["9", "16", "025", "100", "A80"]);
  });

  it("puts this browser's order first, then the rest canonically", () => {
    const rows = ["10", "2", "30", "4"].map((s) => row(s));
    expect(orderRows(rows, ["30", "4"]).map((r) => r.sector_id)).toEqual(["30", "4", "2", "10"]);
  });

  it("moves a row past its visible neighbour and keeps hidden rows in place", () => {
    // 20 is hidden by the filter: moving 30 up lands it above 10, its visible neighbour.
    expect(moveInOrder([], ["10", "20", "30"], ["10", "30"], "30", -1)).toEqual(["30", "10", "20"]);
    expect(moveInOrder([], ["10", "20", "30"], ["10", "30"], "10", 1)).toEqual(["20", "30", "10"]);
    expect(moveInOrder([], ["10", "20"], ["10", "20"], "10", -1)).toBeNull();
    expect(moveInOrder([], ["10", "20"], ["10", "20"], "20", 1)).toBeNull();
  });

  it("judges the alert filter on its own span, not the drawn range", () => {
    const late = row("10", [], ["ok", "ok", "ok", "ok", "ok", "ok", "ok", "ok", "watch"]);
    expect(isAlerting(late, 8)).toBe(false);
    expect(isAlerting(late, 9)).toBe(true);
  });

  it("keys each setting per ARTCC and table", () => {
    expect(storageKey("ZLA", "tracon", "alertSpan")).toBe("ois.sectorMonitor.ZLA.tracon.alertSpan");
  });
});

describe("the consolidation menu's arithmetic (#794, #792)", () => {
  // 12 is worked at 10; 05, 06 and 10 have rows.
  const cons = consolidationOf([row("05"), row("06"), row("10", ["12"])]);

  it("reads the arrangement off the rows and lays pending writes over it", () => {
    expect(cons).toEqual({"12": "10"});
    expect(withPending(cons, {"12": null, "05": "06"})).toEqual({"05": "06"});
  });

  it("offers every other sector not worked elsewhere, and lists what is worked here", () => {
    const universe = ["05", "06", "10", "12"];
    const on06 = menuLists(universe, cons, "06");
    expect(on06.offered).toEqual([
      {sector: "05", checked: false},
      {sector: "10", checked: false},
    ]);
    expect(on06.consolidatedHere).toEqual([]);
    const on10 = menuLists(universe, cons, "10");
    expect(on10.offered.map((s) => s.sector)).toEqual(["05", "06", "12"]);
    expect(on10.consolidatedHere).toEqual([{sector: "12", checked: true}]);
  });

  it("builds the All commands as one patch each", () => {
    const {items} = menuLists(["05", "06", "10", "12"], cons, "06");
    expect(consolidateAllPatch(items, cons, "06", false)).toEqual({"05": "06", "10": "06", "12": "06"});
    expect(consolidateAllPatch(items, cons, "06", true)).toEqual({"05": "06"});
    const two = {...cons, "81": "80"};
    expect(deconsolidateAllPatch(two, "10", "target")).toEqual({"12": null});
    expect(deconsolidateAllPatch(two, "10", "center")).toEqual({"12": null, "81": null});
  });

  it("names the sector in each refusal", () => {
    const known = new Set(["05", "06", "10", "12"]);
    const say = (status: number | undefined, patch: Record<string, string | null>) =>
      consolidationError(status, "ZLA", patch, cons, known);
    expect(say(400, {"25": "25"})).toBe("ZLA25 can't be consolidated into itself.");
    expect(say(409, {"10": "12"})).toBe("Can't consolidate ZLA10 into ZLA12: ZLA12 is worked at ZLA10.");
    expect(say(409, {"05": "06"})).toBe("Can't consolidate ZLA05 into ZLA06: ZLA06 is worked at ZLA05.");
    expect(say(409, {"05": "06", "10": "06"})).toBe("Those consolidations would make a loop; nothing was saved.");
    expect(say(404, {"99": "06"})).toBe("ZLA99 is not one of ZLA's sectors.");
    expect(say(404, {"05": "06"})).toBe("A sector in that change is no longer one of ZLA's sectors; nothing was saved.");
    expect(say(403, {"05": "06"})).toBe("You can't change ZLA's consolidations.");
    for (const status of [500, undefined]) {
      expect(say(status, {"05": "06"})).toBe("Could not save the consolidation — check TMU access / connection.");
    }
  });
});
