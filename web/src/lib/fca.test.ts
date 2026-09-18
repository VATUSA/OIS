import {describe, expect, it} from "vitest";

import {cycleAgeDays, refreshToast, type DataStatus} from "./fca";

/** A cycle `days` old, as the API would report it. */
function cycle(days: number): string {
  return new Date(Date.now() - days * 86_400_000).toISOString().slice(0, 10);
}

function status(over: Partial<DataStatus> = {}): DataStatus {
  return {
    airways: 1,
    fixes: 2,
    nav_cycle: cycle(3),
    nav_refreshed: new Date().toISOString(),
    nav_source: "runtime fetch (faa)",
    navaids: 3,
    procedures: 4,
    winds_refreshed: new Date().toISOString(),
    winds_stations: 176,
    ...over,
  };
}

describe("cycleAgeDays", () => {
  it("measures a cycle's age in whole days", () => {
    expect(cycleAgeDays(cycle(14))).toBe(14);
  });

  it("returns null for anything that isn't a YYYY-MM-DD cycle", () => {
    expect(cycleAgeDays("")).toBeNull();
    expect(cycleAgeDays("2026-9-3")).toBeNull();
    expect(cycleAgeDays("not a cycle")).toBeNull();
  });
});

describe("refreshToast", () => {
  it("reads as success only when nav is current and winds loaded", () => {
    const toast = refreshToast(status());
    expect(toast.variant).toBe("success");
    expect(toast.title).toMatch(/^Nav \d{4}-\d{2}-\d{2} · 176 wind stations$/);
    expect(toast.description).toBeUndefined();
  });

  // The whole point of #332: the endpoint answers 200 even when every upstream is unreachable,
  // because each nav source falls back to the compile-time bundle. Reproduced against a live
  // backend: {"nav_cycle":"2026-07-09","winds_stations":0,"winds_refreshed":null} with HTTP 200.
  // That must not read as success while the stale-NASR banner is still on screen.
  it("warns when the refresh left the cycle stale", () => {
    const toast = refreshToast(status({ nav_cycle: cycle(70), winds_stations: 0 }));
    expect(toast.variant).toBe("warning");
    expect(toast.title).toBe("NASR data is still 70 days old");
    expect(toast.description).toContain("0 wind stations");
  });

  it("warns when nav was never fetched at runtime, even on a fresh cycle", () => {
    const toast = refreshToast(status({ nav_refreshed: null }));
    expect(toast.variant).toBe("warning");
    expect(toast.title).toBe("Nav data could not be fetched");
  });

  it("warns when nav is current but no wind station loaded", () => {
    expect(refreshToast(status({ winds_stations: 0 }))).toMatchObject({
      variant: "warning",
      title: "Winds aloft did not load",
    });
  });

  it("warns when winds were never fetched, even with a non-zero station count", () => {
    expect(refreshToast(status({ winds_refreshed: null })).variant).toBe("warning");
  });

  it("warns when the cycle doesn't parse rather than claiming success", () => {
    const toast = refreshToast(status({ nav_cycle: "garbled" }));
    expect(toast.variant).toBe("warning");
    expect(toast.title).toBe("NASR cycle garbled is unreadable");
  });

  // The boundary the banner uses: at exactly STALE_CYCLE_DAYS the banner is hidden, so the toast
  // must agree and read as success rather than contradicting a screen with no warning on it.
  it("agrees with the banner at the staleness boundary", () => {
    expect(refreshToast(status({ nav_cycle: cycle(35) })).variant).toBe("success");
    expect(refreshToast(status({ nav_cycle: cycle(36) })).variant).toBe("warning");
  });
});
