import type {PickingInfo} from "@deck.gl/core";
import {describe, expect, it} from "vitest";

import type {AtcAnchor} from "../layers/atc";
import type {MatchedFlight} from "../layers/matched";
import {mapTooltip, tooltipFor} from "./tooltip";
import type {NormAircraft} from "./types";

const pick = (layerId: string, object: unknown) => ({ layer: { id: layerId }, object }) as unknown as PickingInfo;
const html = (r: ReturnType<ReturnType<typeof mapTooltip>>) => (r ? r.html : null);

const plane: NormAircraft = {
  id: "AAL1", callsign: "AAL1", actype: "B738", dep: "KDFW", arr: "KORD",
  lat: 40, lon: -80, alt: 35000, gs: 420, heading: 90,
};

const matched: MatchedFlight = {
  callsign: "UAL2", aircraft_type: "A320", seq: 3, heading: 90, lat: 40, lon: -80,
  cross_lat: 41, cross_lon: -79, path: [], dep: "KEWR", arr: "KORD",
  altitude: 24000, groundspeed: 450, distance_nm: 80,
  cross_time: "2026-09-17T14:32:00Z", eta: "2026-09-17T14:28:00Z", delay_sec: 252,
};

const tower: AtcAnchor = {
  type: "airport", icao: "KORD", lat: 41, lon: -87,
  positions: [{ callsign: "ORD_TWR", frequency: "120.750", kind: "TWR", name: "", rating: 3, logon_time: "" }],
};

describe("mapTooltip", () => {
  const tooltip = mapTooltip();

  it("adds sequence, STA, ETA, and delay for an aircraft matched into an FCA", () => {
    const h = html(tooltip(pick("matched", matched)));
    expect(h).toContain("#3");
    expect(h).toContain("STA 1432z");
    expect(h).toContain("ETA 1428z");
    expect(h).toContain("+4:12");
  });

  it("shows on time below the delay threshold", () => {
    expect(html(tooltip(pick("matched", { ...matched, delay_sec: 10 })))).toContain("on time");
  });

  it("keeps plain traffic to the basic card", () => {
    const h = html(tooltip(pick("aircraft", plane)));
    expect(h).toContain("AAL1");
    expect(h).not.toContain("STA");
  });

  it("covers overview mode's per-FCA matched layers", () => {
    expect(html(tooltip(pick("matched-fca123", matched)))).toContain("STA 1432z");
  });

  it("drops only aircraft cards when aircraft tooltips are off", () => {
    const atcOnly = mapTooltip({ aircraft: false });
    expect(atcOnly(pick("aircraft", plane))).toBeNull();
    expect(atcOnly(pick("matched", matched))).toBeNull();
    expect(html(atcOnly(pick("atc-hover", tower)))).toContain("ORD_TWR");
  });
});

describe("tooltipFor", () => {
  it("has no renderer at all when map tooltips are off", () => {
    expect(tooltipFor({ tooltips: false, aircraft: true })).toBeUndefined();
    expect(tooltipFor({ tooltips: false, aircraft: false })).toBeUndefined();
  });

  it("keeps ATC but drops aircraft when only the aircraft toggle is off", () => {
    const t = tooltipFor({ tooltips: true, aircraft: false })!;
    expect(t(pick("aircraft", plane))).toBeNull();
    expect(t(pick("matched", matched))).toBeNull();
    expect(html(t(pick("atc-hover", tower)))).toContain("ORD_TWR");
  });

  it("renders both when both are on", () => {
    const t = tooltipFor({ tooltips: true, aircraft: true })!;
    expect(html(t(pick("aircraft", plane)))).toContain("AAL1");
    expect(html(t(pick("atc-hover", tower)))).toContain("ORD_TWR");
  });
});
