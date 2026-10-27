import {describe, expect, it} from "vitest";

import {type DataStatus, type FcaFlight, fcaIsLive, refreshToast, setResult, windsLabel} from "./fca";

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
    winds_source: "rap",
    ...over,
  };
}

describe("refreshToast", () => {
  it("reads as success only when the loaded cycle is current", () => {
    const toast = refreshToast(status({}));
    expect(toast.variant).toBe("success");
    expect(toast.title).toBe("Nav 2026-09-03 · RAP winds");
    expect(toast.description).toBeUndefined();
  });

  // The production incident: the fetch fell back to the bundle, the endpoint still answered 200, and
  // the toast read "Nav 2026-07-09 · N wind stations" as though nothing were wrong (VATUSA/OIS#317).
  it("warns instead of succeeding when the fetch could only fall back to an older cycle", () => {
    const toast = refreshToast(status({ nav_cycle: "2026-07-09", nav_cycles_behind: 2 }));
    expect(toast.variant).toBe("warning");
    expect(toast.title).toBe("NASR cycle 2026-07-09 is 2 cycles behind");
    expect(toast.description).toBe("Nav 2026-07-09 · RAP winds · current 2026-09-03");
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

// The summary names the wind source the ETAs use, so still air is visible (#836).
describe("windsLabel", () => {
  it("names RAP", () => {
    expect(windsLabel(status({ winds_source: "rap" }))).toBe("RAP winds");
  });

  it("says when there is no wind field", () => {
    expect(windsLabel(status({ winds_source: "none" }))).toBe("no winds");
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

// VATUSA/OIS#816: after a Set, the panel reports how far past the typed ready time the release
// landed, read from the FcaFlight list the mutation returned. Expectations are literals.
function flight(callsign: string, over: Partial<FcaFlight>): FcaFlight {
  return {
    callsign,
    released: true,
    edct: "2026-10-10T14:00:00Z",
    cross_time: "2026-10-10T14:30:00Z",
    ...over,
  } as FcaFlight;
}
const READY = "2026-10-10T14:00:00.000Z";

describe("setResult", () => {
  it("reports the delay as release minus ready in whole minutes", () => {
    const r = setResult([flight("AAL1", { edct: "2026-10-10T14:02:15Z" })], "AAL1", READY);
    expect(r).toEqual({ callsign: "AAL1", ready: READY, delayMin: 2 });
  });

  it("floors to the minute the panel prints, so 14:02:59 is +2 after a 1400Z ready", () => {
    expect(setResult([flight("AAL1", { edct: "2026-10-10T14:02:59Z" })], "AAL1", READY)?.delayMin).toBe(2);
  });

  it("reports zero delay when the release is the ready time", () => {
    expect(setResult([flight("AAL1", {})], "AAL1", READY)?.delayMin).toBe(0);
  });

  it("reads the named callsign, not the first row", () => {
    const rows = [flight("UAL2", { edct: "2026-10-10T14:20:00Z" }), flight("AAL1", { edct: "2026-10-10T14:07:00Z" })];
    expect(setResult(rows, "AAL1", READY)?.delayMin).toBe(7);
  });

  it("is null when the callsign is absent, unreleased or has no release time", () => {
    expect(setResult([flight("UAL2", {})], "AAL1", READY)).toBeNull();
    expect(setResult([flight("AAL1", { released: false })], "AAL1", READY)).toBeNull();
    expect(setResult([flight("AAL1", { edct: null })], "AAL1", READY)).toBeNull();
  });
});
