import type {PickingInfo} from "@deck.gl/core";
import {describe, expect, it} from "vitest";

import {DELAY_THRESHOLD_SEC, fmtDelaySec} from "@/lib/fca";
import type {AtcAnchor} from "../layers/atc";
import type {MatchedFlight} from "../layers/matched";
import {mapTooltip, nearerIsAircraft, tooltipFor} from "./tooltip";
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

/**
 * Like {@link pickOver}, with a viewport that projects `[lon, lat]` straight to pixel `[x, y]`, so a
 * test places the cursor, the aircraft and the pill on one plane (#555).
 */
const pickNear = (layerId: string, object: unknown, under: unknown, cursor: [number, number]) =>
  ({
    ...(pickOver(layerId, object, under) as object),
    x: cursor[0],
    y: cursor[1],
    viewport: { project: ([lon, lat]: [number, number]) => [lon, lat] },
  }) as unknown as PickingInfo;

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

  /** #555: an aircraft on a staffed badge — the nearer of the two wins, so both stay reachable. */
  describe("when an aircraft overlaps an ATC pill", () => {
    // Plane at (100, 100), pill at (110, 100) in pixels.
    const here = { ...plane, lon: 100, lat: 100 };
    const pill = { ...tower, lon: 110, lat: 100 };

    it("shows the aircraft card when the cursor is nearer the aircraft", () => {
      expect(html(tooltip(pickNear("aircraft", here, pill, [102, 100])))).toContain("AAL1");
      expect(
        html(tooltip(pickNear("matched", { ...matched, lon: 100, lat: 100 }, pill, [101, 100]))),
      ).toContain("UAL2");
    });

    it("shows the ATC card when the cursor is nearer the pill", () => {
      expect(html(tooltip(pickNear("aircraft", here, pill, [108, 100])))).toContain("ORD_TWR");
    });

    it("gives a tie to the pill", () => {
      expect(html(tooltip(pickNear("aircraft", here, pill, [105, 100])))).toContain("ORD_TWR");
    });

    it("shows the pill when aircraft cards are off, however near the aircraft", () => {
      const atcOnly = mapTooltip({ aircraft: false });
      expect(html(atcOnly(pickNear("aircraft", here, pill, [100, 100])))).toContain("ORD_TWR");
    });
  });

  it("measures distance in screen pixels from the cursor", () => {
    expect(nearerIsAircraft([0, 0], [3, 4], [6, 0])).toBe(true);
    expect(nearerIsAircraft([0, 0], [6, 0], [3, 4])).toBe(false);
    expect(nearerIsAircraft([0, 0], [5, 0], [0, 5])).toBe(false);
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
