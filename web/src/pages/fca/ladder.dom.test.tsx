// @vitest-environment jsdom
//
// VATUSA/OIS#814 AC6 on the FCA ladder: a tag whose crossing rests on the flight's proposed wheels-up is
// marked, in the classic style and in TGUI's.
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {afterEach, beforeAll, describe, expect, it} from "vitest";

import type {FcaFlight} from "@/lib/fca";
import {Ladder, tguiColumn} from "./ladder";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
});

const roots: { root: ReturnType<typeof createRoot>; host: HTMLElement }[] = [];
afterEach(() => {
  for (const { root, host } of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
});

const NOW = Date.parse("2026-10-03T12:00:00Z");
const at = (min: number) => new Date(NOW + min * 60_000).toISOString();

const flight = (callsign: string, cross: number, over: Partial<FcaFlight>) =>
  ({
    callsign,
    status: "ground",
    seq: cross,
    cross_time: at(cross),
    eta: at(cross),
    delay_sec: 0,
    released: false,
    wheels_up_proposed: false,
    ...over,
  }) as FcaFlight;

const FLIGHTS = [
  flight("GRD1", 10, { wheels_up_proposed: true }),
  flight("REL1", 20, { released: true }),
];

function mount(style: string) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, refetchOnMount: false, staleTime: Infinity } },
  });
  qc.setQueryData(["preferences", "settings"], { "ladder.style": style });
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({ root, host });
  act(() =>
    root.render(
      <QueryClientProvider client={qc}>
        <Ladder flights={FLIGHTS} now={NOW} name="FCA" />
      </QueryClientProvider>,
    ),
  );
  return host;
}

/** The smallest element whose text holds `callsign`: its tag. */
function tag(host: HTMLElement, callsign: string, selector: string) {
  const found = [...host.querySelectorAll<HTMLElement>(selector)].filter((el) =>
    el.textContent?.includes(callsign),
  );
  expect(found.length, `${callsign} has a tag`).toBeGreaterThan(0);
  return found.at(-1)!;
}

describe("FCA Ladder proposed marker (#814)", () => {
  it("marks a proposed flight's tag on the classic ladder", () => {
    const host = mount("classic");
    expect(host.querySelector('[data-ladder="tgui"]')).toBeNull();
    const grd = tag(host, "GRD1", "span.rounded-xs");
    expect(grd.querySelector(".sr-only")?.textContent).toBe(" proposed");
    const rel = tag(host, "REL1", "span.rounded-xs");
    expect(rel.querySelector(".sr-only")).toBeNull();
  });

  it("marks a proposed flight on the TGUI ladder", () => {
    expect(tguiColumn(FLIGHTS, NOW, "FCA").items.map((i) => [i.callsign, i.proposed])).toEqual([
      ["GRD1", true],
      ["REL1", false],
    ]);
    const host = mount("tgui");
    expect(host.querySelector('[data-ladder="tgui"]')).not.toBeNull();
    const grd = host.querySelectorAll('[data-tag][data-key="GRD1"]');
    expect(grd.length, "positive control: GRD1 is drawn").toBeGreaterThan(0);
    for (const t of grd) expect(t.querySelector(".sr-only")?.textContent).toBe(" proposed");
    for (const t of host.querySelectorAll('[data-tag][data-key="REL1"]')) expect(t.querySelector(".sr-only")).toBeNull();
  });
});
