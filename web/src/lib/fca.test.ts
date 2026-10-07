import {describe, expect, it} from "vitest";

import {type DataStatus, fcaIsLive, refreshToast} from "./fca";

function status(over: Partial<DataStatus>): DataStatus {
  return {
    airways: 1,
    fixes: 2,
    nav_cycle: "2026-09-03",
    nav_cycle_current: "2026-09-03",
    nav_cycles_behind: 0,
    nav_source: "runtime fetch (faa)",
    navaids: 3,
    procedures: 4,
    winds_stations: 176,
    ...over,
  };
}

describe("refreshToast", () => {
  it("reads as success only when the loaded cycle is current", () => {
    const toast = refreshToast(status({}));
    expect(toast.variant).toBe("success");
    expect(toast.title).toBe("Nav 2026-09-03 · 176 wind stations");
    expect(toast.description).toBeUndefined();
  });

  // The production incident: the fetch fell back to the bundle, the endpoint still answered 200, and
  // the toast read "Nav 2026-07-09 · N wind stations" as though nothing were wrong (VATUSA/OIS#317).
  it("warns instead of succeeding when the fetch could only fall back to an older cycle", () => {
    const toast = refreshToast(status({ nav_cycle: "2026-07-09", nav_cycles_behind: 2 }));
    expect(toast.variant).toBe("warning");
    expect(toast.title).toBe("NASR cycle 2026-07-09 is 2 cycles behind");
    expect(toast.description).toBe("Nav 2026-07-09 · 176 wind stations · current 2026-09-03");
  });

  it("says one cycle behind in the singular", () => {
    expect(refreshToast(status({ nav_cycle: "2026-08-06", nav_cycles_behind: 1 })).title).toBe(
      "NASR cycle 2026-08-06 is 1 cycle behind",
    );
  });

  it("warns when the loaded cycle isn't a readable date, whether null or absent", () => {
    const explicit = refreshToast(status({ nav_cycle: "unknown", nav_cycles_behind: null }));
    expect(explicit.variant).toBe("warning");
    expect(explicit.title).toBe("NASR cycle unknown is unreadable");

    const absent = refreshToast(status({ nav_cycle: "unknown", nav_cycles_behind: undefined }));
    expect(absent.variant).toBe("warning");
    expect(absent.title).toBe("NASR cycle unknown is unreadable");
  });
});

// The live-operation routes (release, swap, reorder) 404 on an event FCA that isn't published, to
// every caller (VATUSA/OIS#746), so only a published event FCA or an ordinary one is live.
describe("fcaIsLive", () => {
  it.each([
    [null, null, true],
    [7461, "published", true],
    [7461, "planned", false],
    [7461, "archived", false],
    [7461, null, false],
  ])("event %s / %s → %s", (eventId, eventStatus, live) => {
    expect(fcaIsLive({event_id: eventId, event_status: eventStatus})).toBe(live);
  });
});
