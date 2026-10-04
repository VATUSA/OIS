import {describe, expect, it} from "vitest";

import {type IdstFlight, swapPartners} from "./idst";

const flight = (callsign: string, over: Partial<IdstFlight> = {}): IdstFlight => ({
  callsign,
  dep: "KJFK",
  arr: "KDCA",
  aircraft_type: "B738",
  status: "ground",
  fca_id: "f1",
  fca_name: "ZDC WEST",
  seq: 1,
  delay_min: 0,
  cross_time: "2026-10-03T12:30:00Z",
  edct: "2026-10-03T12:10:00Z",
  released: true,
  released_by_machine: null,
  runway: "31L",
  runway_source: "config",
  ...over,
});

/** #56: the IDST swap picker offers only flights the server would let trade. */
describe("swapPartners", () => {
  const me = flight("AAL1");

  it("offers released flights on the same FCA, airport and runway, never itself", () => {
    const same = flight("UAL2");
    const out = swapPartners(me, [me, same]);
    expect(out.map((f) => f.callsign)).toEqual(["UAL2"]);
  });

  it("leaves out another runway, airport or FCA, and unreleased flights", () => {
    const released = [
      flight("RWY4", { runway: "4L" }),
      flight("LGA1", { dep: "KLGA" }),
      flight("FCA2", { fca_id: "f2" }),
      flight("WAIT", { released: false }),
      flight("NORW", { runway: null }),
    ];
    expect(swapPartners(me, released)).toEqual([]);
  });

  it("matches the departure airport case-insensitively", () => {
    expect(swapPartners(me, [flight("UAL2", { dep: "kjfk" })])).toHaveLength(1);
  });

  it("offers nothing for a flight with no runway or no release", () => {
    const peer = flight("UAL2");
    expect(swapPartners(flight("AAL1", { runway: null }), [peer])).toEqual([]);
    expect(swapPartners(flight("AAL1", { released: false }), [peer])).toEqual([]);
  });
});
