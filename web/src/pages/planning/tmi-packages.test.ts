// VATUSA/OIS#537: advisory items in an event TMI package.
//
// `itemSummary`'s trailing `return` assumed ground stop, and `submit`'s final branch was a bare
// `else` that also assumed it — so adding a kind to the union silently made it a ground stop in two
// places. These pin the advisory case rather than the fallthrough.
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";

import {buildItemPayload, itemSummary} from "./tmi-packages";
import type {TmiPackageItem} from "@/lib/events";
import {EMPTY_NTML, type Ntml} from "@/lib/ntml";

const item = (kind: string, payload: Record<string, unknown>) =>
  ({id: "i1", kind, payload} as unknown as TmiPackageItem);

describe("itemSummary for an advisory (VATUSA/OIS#537)", () => {
  it("names the facility and document type", () => {
    const s = itemSummary(
      item("advisory", {
        facility: "DCC",
        kind: "reroute",
        valid_from: "2026-10-02T14:00:00Z",
        valid_to: "2026-10-02T18:00:00Z",
      }),
    );

    expect(s).toContain("DCC");
    expect(s).toContain("reroute");
  });

  it("shows the validity window", () => {
    const s = itemSummary(
      item("advisory", {
        facility: "DCC",
        kind: "gdp",
        valid_from: "2026-10-02T14:00:00Z",
        valid_to: "2026-10-02T18:00:00Z",
      }),
    );

    expect(s).toContain("–"); // a range, not one timestamp
    expect(s.match(/14|18/), "the window's hours appear").not.toBeNull();
  });

  /**
   * The regression this guards. A ground stop summarises as `airport · scope · until`; an advisory
   * must not be rendered through that branch, which would print "· all · UFN" for a document that
   * has neither.
   */
  it("is not summarised as a ground stop", () => {
    const s = itemSummary(item("advisory", {facility: "DCC", kind: "reroute"}));

    expect(s).not.toContain("UFN");
    expect(s).not.toContain("all");
  });

  /** And a real ground stop still summarises as one. */
  it("leaves the ground-stop summary alone", () => {
    expect(itemSummary(item("ground_stop", {airport: "KDCA"}))).toContain("UFN");
  });

  /** An advisory with no window still names itself rather than rendering a stray separator. */
  it("omits the window when there is none", () => {
    const s = itemSummary(item("advisory", {facility: "DCC", kind: "reroute"}));

    expect(s).toContain("DCC");
    expect(s.endsWith("·") || s.endsWith(" · ")).toBe(false);
  });
});

// VATUSA/OIS#1012: every check names the field it refuses, and a restriction start or stop that does
// not parse is refused instead of being dropped. Each refusal breaks one field of a fixture that
// otherwise passes every check, so only the named check can refuse it.
describe("buildItemPayload (VATUSA/OIS#1012)", () => {
  // `parseZulu` resolves DD/HHMMz against the current month.
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-10-12T12:00:00Z"));
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  const program = {icao: "KJFK", aar: "40", trail: "10", mit: "20"};
  const restriction = {requesting: "ZNY", providing: "ZDC", restriction: "20 MIT", start: "12/1430z", stop: "12/1830z"};
  const advisory = {facility: "DCC", advKind: "reroute", body: "ROUTE TEXT", validFrom: "12/1430z", validTo: "12/1830z"};
  const ntml: Ntml = {...EMPTY_NTML, element: "JFK", kind: "MIT"};

  it('refuses a program with no ICAO: "ICAO is required"', () => {
    expect(buildItemPayload("program", {...program, icao: ""}, false, EMPTY_NTML)).toEqual({error: "ICAO is required"});
  });

  it('refuses a program with no AAR: "AAR is required"', () => {
    expect(buildItemPayload("program", {...program, aar: ""}, false, EMPTY_NTML)).toEqual({error: "AAR is required"});
  });

  it("builds a program payload from ICAO, AAR, MINIT and MIT", () => {
    expect(buildItemPayload("program", program, false, EMPTY_NTML)).toEqual({
      payload: {icao: "KJFK", aar: 40, trail: 10, mit: 20},
    });
  });

  it('refuses a restriction with no requesting facility: "Requesting is required"', () => {
    expect(buildItemPayload("restriction", {...restriction, requesting: ""}, false, EMPTY_NTML)).toEqual({
      error: "Requesting is required",
    });
  });

  it('refuses a restriction with no providing facility: "Providing is required"', () => {
    expect(buildItemPayload("restriction", {...restriction, providing: ""}, false, EMPTY_NTML)).toEqual({
      error: "Providing is required",
    });
  });

  it('refuses a free-text restriction with no restriction text: "A restriction is required"', () => {
    expect(buildItemPayload("restriction", {...restriction, restriction: ""}, false, EMPTY_NTML)).toEqual({
      error: "A restriction is required",
    });
  });

  it("refuses a structured restriction with a blank element", () => {
    expect(buildItemPayload("restriction", {...restriction, restriction: ""}, true, {...ntml, element: "  "})).toEqual({
      error: "An element and a restriction type are required",
    });
  });

  it("refuses a structured restriction with a blank restriction type", () => {
    expect(buildItemPayload("restriction", {...restriction, restriction: ""}, true, {...ntml, kind: ""})).toEqual({
      error: "An element and a restriction type are required",
    });
  });

  const unparseable = ["1430z", "12/2430z", "32/1430z", "12-1430z", "tomorrow"];

  it('refuses a restriction whose start does not parse: "Start time must be DD/HHMMz (e.g. 12/1430z)"', () => {
    for (const start of unparseable) {
      expect(buildItemPayload("restriction", {...restriction, start}, false, EMPTY_NTML), `start ${start}`).toEqual({
        error: "Start time must be DD/HHMMz (e.g. 12/1430z)",
      });
    }
  });

  it('refuses a restriction whose stop does not parse: "Stop time must be DD/HHMMz (e.g. 12/1830z)"', () => {
    for (const stop of unparseable) {
      expect(buildItemPayload("restriction", {...restriction, stop}, false, EMPTY_NTML), `stop ${stop}`).toEqual({
        error: "Stop time must be DD/HHMMz (e.g. 12/1830z)",
      });
    }
  });

  it("builds a restriction with ISO start_time and stop_time from parseable DD/HHMMz entries", () => {
    expect(buildItemPayload("restriction", restriction, false, EMPTY_NTML)).toEqual({
      payload: {
        requesting: "ZNY",
        providing: "ZDC",
        restriction: "20 MIT",
        structured: undefined,
        start_time: "2026-10-12T14:30:00.000Z",
        stop_time: "2026-10-12T18:30:00.000Z",
      },
    });
  });

  it("builds a restriction with no window when start and stop are blank, absent or whitespace", () => {
    const cases: Record<string, string>[] = [
      {...restriction, start: "", stop: ""},
      {requesting: "ZNY", providing: "ZDC", restriction: "20 MIT"},
      {...restriction, start: "   ", stop: "  "},
    ];
    for (const f of cases) {
      const built = buildItemPayload("restriction", f, false, EMPTY_NTML);
      expect(built, JSON.stringify(f)).toEqual({
        payload: {
          requesting: "ZNY",
          providing: "ZDC",
          restriction: "20 MIT",
          structured: undefined,
          start_time: undefined,
          stop_time: undefined,
        },
      });
    }
  });

  it("builds a structured restriction carrying the ntml and an empty restriction string", () => {
    const built = buildItemPayload("restriction", {...restriction, restriction: ""}, true, ntml);
    expect(built).toMatchObject({payload: {structured: ntml, restriction: ""}});
  });

  it('refuses a ground stop with no airport: "Airport is required"', () => {
    expect(buildItemPayload("ground_stop", {airport: ""}, false, EMPTY_NTML)).toEqual({error: "Airport is required"});
  });

  it("builds a ground stop payload, omitting blank scope and until", () => {
    expect(buildItemPayload("ground_stop", {airport: "KDCA", scope: "", until: ""}, false, EMPTY_NTML)).toEqual({
      payload: {airport: "KDCA", scope: undefined, until: undefined},
    });
  });

  it('refuses an advisory with no facility: "Facility is required"', () => {
    expect(buildItemPayload("advisory", {...advisory, facility: ""}, false, EMPTY_NTML)).toEqual({
      error: "Facility is required",
    });
  });

  it('refuses an advisory with no advisory type: "Advisory type is required"', () => {
    expect(buildItemPayload("advisory", {...advisory, advKind: ""}, false, EMPTY_NTML)).toEqual({
      error: "Advisory type is required",
    });
  });

  it("refuses an advisory whose valid-from is blank or does not parse", () => {
    for (const validFrom of ["", "12/2500z"]) {
      expect(buildItemPayload("advisory", {...advisory, validFrom}, false, EMPTY_NTML), `from ${validFrom}`).toEqual({
        error: "Valid from must be DD/HHMMz (e.g. 12/1430z)",
      });
    }
  });

  it("refuses an advisory whose valid-to is blank or does not parse", () => {
    for (const validTo of ["", "12/2500z"]) {
      expect(buildItemPayload("advisory", {...advisory, validTo}, false, EMPTY_NTML), `to ${validTo}`).toEqual({
        error: "Valid to must be DD/HHMMz (e.g. 12/1830z)",
      });
    }
  });

  it('refuses an advisory whose valid-to is not after valid-from: "Valid to must be after valid from"', () => {
    for (const validTo of ["12/1430z", "12/1429z"]) {
      expect(buildItemPayload("advisory", {...advisory, validTo}, false, EMPTY_NTML), `to ${validTo}`).toEqual({
        error: "Valid to must be after valid from",
      });
    }
  });

  it('refuses an advisory with no advisory text: "Advisory text is required"', () => {
    expect(buildItemPayload("advisory", {...advisory, body: ""}, false, EMPTY_NTML)).toEqual({
      error: "Advisory text is required",
    });
  });

  it("builds an advisory one minute long with ISO valid_from and valid_to", () => {
    expect(buildItemPayload("advisory", {...advisory, validTo: "12/1431z"}, false, EMPTY_NTML)).toEqual({
      payload: {
        facility: "DCC",
        kind: "reroute",
        body: "ROUTE TEXT",
        valid_from: "2026-10-12T14:30:00.000Z",
        valid_to: "2026-10-12T14:31:00.000Z",
      },
    });
  });
});
