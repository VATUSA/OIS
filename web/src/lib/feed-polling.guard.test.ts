import {readFileSync} from "node:fs";
import {describe, expect, it} from "vitest";

/**
 * VATUSA/OIS#648: every hook behind a feed-derived query polls only while feed ticks are not
 * arriving. `pollUnlessLive` itself is unit-tested; this pins that each hook actually goes through it,
 * since a hook drifting back to a bare `refetchInterval: 15_000` would poll on top of the tick and
 * pass every other test.
 */
const HOOKS: [file: string, hook: string][] = [
  ["fca.ts", "useFcaCounts"],
  ["fca.ts", "useTraffic"],
  ["fca.ts", "useAtc"],
  ["fca.ts", "useFcaTraffic"],
  ["fca.ts", "useFcaTrafficMany"],
  ["feed.ts", "useFeedStatus"],
  ["feed.ts", "useAirportFlow"],
  ["feed.ts", "useMultiAirportFlow"],
  ["taxi.ts", "useTaxiStats"],
  ["aadc.ts", "useAadc"],
  ["idst.ts", "useIdst"],
  ["departures.ts", "useDepartures"],
  ["departures.ts", "useMultiDepartures"],
];

function body(file: string, hook: string): string {
  const src = readFileSync(new URL(`./${file}`, import.meta.url), "utf8");
  const start = src.indexOf(`export function ${hook}(`);
  expect(start, `${file} exports ${hook}`).toBeGreaterThanOrEqual(0);
  const next = src.indexOf("\nexport function ", start + 1);
  return src.slice(start, next === -1 ? undefined : next);
}

describe("feed-derived hooks poll only without ticks (#648)", () => {
  it.each(HOOKS)("%s %s", (file, hook) => {
    const fn = body(file, hook);
    expect(fn).toContain("const live = useRealtimeLive();");
    const intervals = fn.match(/refetchInterval:[^\n]*/g) ?? [];
    expect(intervals.length, "one interval").toBe(1);
    expect(intervals[0]).toContain("pollUnlessLive(");
  });
});
