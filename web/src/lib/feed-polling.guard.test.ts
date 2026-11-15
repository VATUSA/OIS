import {readFileSync} from "node:fs";
import {describe, expect, it} from "vitest";

/**
 * VATUSA/OIS#648: every hook behind a feed-derived query polls only while feed ticks are not
 * arriving. `pollUnlessLive` itself is unit-tested; this pins that each hook actually goes through it,
 * since a hook drifting back to a bare `refetchInterval: 15_000` would poll on top of the tick and
 * pass every other test. The interval must pass `live` itself and keep the hook's fallback spacing
 * (#954): `pollUnlessLive(60_000, false)` or a changed interval would pass a bare "goes through it" check.
 */
const HOOKS: [file: string, hook: string, fallbackMs: number][] = [
  ["fca.ts", "useFcaCounts", 15_000],
  ["fca.ts", "useTraffic", 15_000],
  ["fca.ts", "useAtc", 15_000],
  ["fca.ts", "useFcaTraffic", 30_000],
  ["fca.ts", "useFcaTrafficMany", 30_000],
  ["feed.ts", "useFeedStatus", 30_000],
  ["feed.ts", "useAirportFlow", 20_000],
  ["feed.ts", "useMultiAirportFlow", 20_000],
  ["taxi.ts", "useTaxiStats", 15_000],
  ["aadc.ts", "useAadc", 20_000],
  ["idst.ts", "useIdst", 30_000],
  ["departures.ts", "useDepartures", 60_000],
  ["departures.ts", "useMultiDepartures", 60_000],
  ["tmu.ts", "useAirportDemand", 60_000],
];

function body(file: string, hook: string): string {
  const src = readFileSync(new URL(`./${file}`, import.meta.url), "utf8");
  const start = src.indexOf(`export function ${hook}(`);
  expect(start, `${file} exports ${hook}`).toBeGreaterThanOrEqual(0);
  const next = src.indexOf("\nexport function ", start + 1);
  return src.slice(start, next === -1 ? undefined : next);
}

describe("feed-derived hooks poll only without ticks (#648)", () => {
  it.each(HOOKS)("%s %s polls every %ims without ticks", (file, hook, fallbackMs) => {
    const fn = body(file, hook);
    expect(fn).toContain("const live = useRealtimeLive();");
    const intervals = fn.match(/refetchInterval:[^\n]*/g) ?? [];
    expect(intervals.length, "one interval").toBe(1);
    const call = (intervals[0] ?? "").match(/pollUnlessLive\(\s*([\d_]+)\s*,\s*live\s*\)/);
    expect(call, `${hook} polls through pollUnlessLive(<ms>, live)`).not.toBeNull();
    expect(Number(call![1].replaceAll("_", ""))).toBe(fallbackMs);
  });
});
