import {describe, expect, it} from "vitest";

import type {FcaFlight} from "@/lib/fca";
import {ladderItems} from "./ladder";

const NOW = new Date("2026-09-27T12:00:00Z").getTime();
const flight = (callsign: string, status: string, crossIn: number | null) =>
  ({
    callsign,
    status,
    seq: 1,
    cross_time: crossIn == null ? null : new Date(NOW + crossIn * 60_000).toISOString(),
  }) as unknown as FcaFlight;

describe("ladderItems", () => {
  // The pop-out passed raw traffic and plotted prefiled flights its source panel hides, so the two
  // showed different sequences (VATUSA/OIS#349 review). The ladder itself leaves them off now.
  it("plots connected traffic and leaves proposed flights off", () => {
    const items = ladderItems(
      [flight("AAL1", "airborne", 10), flight("PRE1", "proposed", 12), flight("DAL2", "ground", 30)],
      NOW,
    );
    expect(items.map((i) => i.key)).toEqual(["AAL1", "DAL2"]);
  });

  it("drops a flight with no crossing time", () => {
    expect(ladderItems([flight("UAL3", "airborne", null)], NOW)).toEqual([]);
  });
});
