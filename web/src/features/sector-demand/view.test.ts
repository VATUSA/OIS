import {describe, expect, it} from "vitest";

import type {SectorDemandBin, SectorDemandRow} from "./sector-demand";
import {DEFAULT_VIEW, SPAN_CHOICES_H, formatHours, isAlerting, loadView, visibleGrid} from "./view";

const START = Date.UTC(2026, 9, 7, 14, 0);
const BINS = Array.from({ length: 24 }, (_, i) => START + i * 15 * 60_000);

const ok: SectorDemandBin = { active: 1, proposed: 0, combined: 1, level: "ok" };

/** A row green everywhere except `level` at bin `at`. */
function row(id: string, at?: number, level: SectorDemandBin["level"] = "watch"): SectorDemandRow {
  return {
    sector_id: id,
    tier: "high",
    limit: 10,
    limit_overridden: false,
    consolidated: [],
    bins: BINS.map((_, i) => (i === at ? { active: 9, proposed: 3, combined: 12, level } : ok)),
  };
}

describe("isAlerting", () => {
  it("looks only at the span's bins: an alert at 1h45 is inside 2 h and outside 1.5 h", () => {
    const r = row("05", 7);
    expect(isAlerting(r, 2)).toBe(true);
    expect(isAlerting(r, 1.5)).toBe(false);
    // The last bin of a span counts; the first bin past it does not.
    expect(isAlerting(row("05", 6), 1.75)).toBe(true);
    expect(isAlerting(row("05", 7), 1.75)).toBe(false);
  });

  it("treats red and yellow alike, and an all-green row as quiet", () => {
    expect(isAlerting(row("05", 0, "over"), 0.25)).toBe(true);
    expect(isAlerting(row("05", 0, "watch"), 0.25)).toBe(true);
    expect(isAlerting(row("05"), 6)).toBe(false);
  });
});

describe("visibleGrid", () => {
  it("cuts every row and the time axis to the drawn range", () => {
    const all = { ...DEFAULT_VIEW, alertOnly: false };
    const g = visibleGrid([row("05")], BINS, { ...all, rangeH: 2 });
    expect(g.binStarts).toEqual(BINS.slice(0, 8));
    expect(g.rows[0].cells).toHaveLength(8);
    expect(visibleGrid([row("05")], BINS, { ...all, rangeH: 6 }).rows[0].cells).toHaveLength(24);
  });

  it("filters on its own span, independent of the range", () => {
    // Alerting at 5h00: past a 4-hour range, inside a 6-hour span.
    const late = row("LATE", 20);
    const quiet = row("QUIET");
    const view = { rangeH: 4, alertOnly: true, alertSpanH: 6 };
    expect(visibleGrid([late, quiet], BINS, view).rows.map((r) => r.id)).toEqual(["LATE"]);
    // Alerting at 0h30: inside a 1.5 h span on a 6-hour table, and hidden by a 0.25 h span.
    const soon = row("SOON", 2);
    expect(visibleGrid([soon, quiet], BINS, { rangeH: 6, alertOnly: true, alertSpanH: 1.5 }).rows.map((r) => r.id)).toEqual([
      "SOON",
    ]);
    expect(visibleGrid([soon], BINS, { rangeH: 6, alertOnly: true, alertSpanH: 0.25 }).rows).toEqual([]);
  });

  it("hides nothing while the filter is off", () => {
    expect(visibleGrid([row("A"), row("B", 3)], BINS, { ...DEFAULT_VIEW, alertOnly: false }).rows).toHaveLength(2);
  });

  it("shows only what alerts in the next 2 h by default", () => {
    // B alerts at 0h45, C at 1h45 (the span's last bin), D at 2h00 (the first bin past it).
    const rows = [row("A"), row("B", 3), row("C", 7), row("D", 8)];
    expect(visibleGrid(rows, BINS, DEFAULT_VIEW).rows.map((r) => r.id)).toEqual(["B", "C"]);
  });

  it("carries a combined row's sources and the server's levels through untouched", () => {
    const combined = { ...row("05", 1, "over"), consolidated: ["06", "07"], name: "SHENANDOAH" };
    const [r] = visibleGrid([combined], BINS, DEFAULT_VIEW).rows;
    expect(r.carries).toEqual(["06", "07"]);
    expect(r.name).toBe("SHENANDOAH");
    expect(r.cells[1]).toEqual({ combined: 12, active: 9, level: "over" });
  });
});

describe("controls", () => {
  it("formats hours the way the empty-filter message reads", () => {
    expect(formatHours(2)).toBe("2.00 h");
    expect(formatHours(1.5)).toBe("1.50 h");
  });

  it("offers every quarter-hour span from 0.25 h to 6 h", () => {
    expect(SPAN_CHOICES_H[0]).toBe(0.25);
    expect(SPAN_CHOICES_H.at(-1)).toBe(6);
    expect(SPAN_CHOICES_H).toHaveLength(24);
  });

  it("defaults to a 4-hour range, filtered to sectors alerting in the next 2 hours", () => {
    expect(DEFAULT_VIEW).toEqual({ rangeH: 4, alertOnly: true, alertSpanH: 2 });
    // No browser storage here at all: the defaults, not a throw.
    expect(loadView("ZDC", "enroute")).toEqual(DEFAULT_VIEW);
  });
});
