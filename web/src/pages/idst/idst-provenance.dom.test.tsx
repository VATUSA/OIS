// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, describe, expect, it} from "vitest";

import type {IdstFlight} from "@/lib/idst";

import {FlightTable} from "./index";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

const roots: { root: ReturnType<typeof createRoot>; host: HTMLElement }[] = [];
afterEach(() => {
  for (const { root, host } of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
});

const flight = (callsign: string, released_by_machine: string | null): IdstFlight => ({
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
  released_by_machine,
  runway: null,
  runway_source: null,
});

/** VATUSA/OIS#585 AC4: a controller can see a release came from a tool, on the real IDST table. */
describe("IDST release provenance", () => {
  it("names the tool beside a machine-issued release, and nothing beside a person's", () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    roots.push({ root, host });
    act(() =>
      root.render(
        <FlightTable
          title="Released"
          flights={[flight("AAL1", "vTBFM"), flight("UAL2", null)]}
          selKey={null}
          onSelect={() => {}}
          empty="none"
        />,
      ),
    );

    const marks = [...host.querySelectorAll("[data-release-source]")];
    expect(marks).toHaveLength(1);
    expect(marks[0].textContent).toContain("via vTBFM");
    const row = marks[0].closest("tr");
    expect(row?.textContent).toContain("AAL1");
  });
});
