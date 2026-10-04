import type {PickingInfo} from "@deck.gl/core";
import {describe, expect, it} from "vitest";

import {DELAY_THRESHOLD_SEC, fmtDelaySec} from "@/lib/fca";
import type {AtcAnchor} from "../layers/atc";
import type {MatchedFlight} from "../layers/matched";
import {flightLevel, mapTooltip, sectorHtml, sectorTooltip, tooltipFor} from "./tooltip";
import type {NormAircraft} from "./types";

const pick = (layerId: string, object: unknown) => ({ layer: { id: layerId }, object }) as unknown as PickingInfo;

/** A pick on `layerId` with `under` sitting beneath it in another layer (deck re-pick reachable). */
const pickOver = (layerId: string, object: unknown, under: unknown) =>
  ({
    x: 10,
    y: 20,
    object,
    layer: { id: layerId, context: { deck: { pickObject: () => (under ? { object: under } : null) } } },
  }) as unknown as PickingInfo;
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

  // Pins the boundary itself: a delay exactly at the threshold is flagged, one second under is not.
  it("flags a delay exactly at the threshold but not one second below it", () => {
    const at = html(tooltip(pick("matched", { ...matched, delay_sec: DELAY_THRESHOLD_SEC })));
    expect(at).toContain(`+${fmtDelaySec(DELAY_THRESHOLD_SEC)}`);
    expect(at).not.toContain("on time");
    expect(
      html(tooltip(pick("matched", { ...matched, delay_sec: DELAY_THRESHOLD_SEC - 1 }))),
    ).toContain("on time");
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

  // A plane parked on a staffed airport's badge wins the pick. With aircraft cards off, the pill
  // underneath must still get its card rather than the hover going dead (#323, AC3).
  it("still shows the ATC pill under a glyph when aircraft tooltips are off", () => {
    const atcOnly = mapTooltip({ aircraft: false });
    const overPill = pickOver("aircraft", plane, tower);
    expect(html(atcOnly(overPill))).toContain("ORD_TWR");
    // Nothing underneath — no card, rather than an empty one.
    expect(atcOnly(pickOver("aircraft", plane, null))).toBeNull();
  });

  // VATUSA/OIS#477 symptom 3: the same rescue has to work with **default** settings. The aircraft
  // IconLayer is pushed above `atc-hover` and is far bigger than the ATC circle, so at a staffed
  // airport the glyph wins the pick — and #323 applied the re-pick only when aircraft cards were
  // off, which left the defaulted configuration the one path that could never show an ATC card.
  //
  // This is the pick-resolution rule, so it is also the regression guard for the layer ordering:
  // whatever the layer stack does, hovering a pill must resolve to the pill.
  it("shows the ATC pill under a glyph with default settings (aircraft tooltips on)", () => {
    expect(html(tooltip(pickOver("aircraft", plane, tower)))).toContain("ORD_TWR");
    expect(html(tooltip(pickOver("matched", matched, tower)))).toContain("ORD_TWR");
  });

  it("still shows the aircraft card when no pill is underneath", () => {
    // The pill only wins where there actually is one; an aircraft in open airspace is unaffected.
    expect(html(tooltip(pickOver("aircraft", plane, null)))).toContain("AAL1");
    expect(html(tooltip(pickOver("matched", matched, null)))).toContain("UAL2");
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

describe("sectorTooltip (#602)", () => {
  const volume = {
    artcc: "ZDC",
    sector_id: "32",
    volume_id: "03201",
    name: "Gordonsville 32",
    tier: "high",
    base_alt_ft: 24_000,
    top_alt_ft: 35_000,
    rings: [],
  };

  it("names the sector, its tier and vertical band", () => {
    const card = sectorTooltip()(pick("airspace-sectors", { volume }));
    expect(card?.html).toContain("ZDC 32 · High");
    expect(card?.html).toContain("Gordonsville 32");
    expect(card?.html).toContain("FL240–FL350");
  });

  /** The name comes from imported data and the card is rendered as html. */
  it("escapes a sector name that contains markup", () => {
    const html = sectorHtml({ ...volume, name: '<img src=x onerror="alert(1)">' });
    expect(html).not.toContain("<img");
    expect(html).toContain("&lt;img src=x onerror=&quot;alert(1)&quot;&gt;");
  });

  it("answers only for the sector layer", () => {
    expect(sectorTooltip()(pick("surface-gates", { volume }))).toBeNull();
    expect(sectorTooltip()(pick("airspace-sectors", {}))).toBeNull();
  });

  it("calls a floor at the surface SFC and pads flight levels", () => {
    expect(flightLevel(0)).toBe("SFC");
    expect(flightLevel(5_000)).toBe("FL050");
    expect(flightLevel(60_000)).toBe("FL600");
  });
});
