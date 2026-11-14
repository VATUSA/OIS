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
  // The dashboard's mode-aware hooks: only the live branch polls; the replay branch carries no
  // interval (#952).
  ["historical.ts", "useModeAirportFlow"],
  ["historical.ts", "useModeDepartures"],
  ["historical.ts", "useModeTaxi"],
  ["historical.ts", "useModeTraffic"],
];

function body(file: string, hook: string): string {
  const src = readFileSync(new URL(`./${file}`, import.meta.url), "utf8");
  const start = src.indexOf(`export function ${hook}(`);
  expect(start, `${file} exports ${hook}`).toBeGreaterThanOrEqual(0);
  const next = src.indexOf("\nexport function ", start + 1);
  return src.slice(start, next === -1 ? undefined : next);
}

// The whole interval line, anchored, so the call is tied to the live branch and passed `live`.
// `pollUnlessLive(15_000, false)` and `… || 15_000` poll on top of the tick; a swapped ternary stops
// the live fallback poll, and a polling replay branch polls a `hist-*` key no tick refetches (#952).
const LIVE_ONLY = [/^refetchInterval: pollUnlessLive\([\d_]+, live\),$/];
const MODE_AWARE = [
  // useQuery: the live branch of an `at == null` ternary; the replay branch is `false`.
  /^refetchInterval: at == null \? pollUnlessLive\([\d_]+, live\) : false,$/,
  // useQueries: the live branch's options object, on the `? {` line of the `at == null` ternary.
  /^\? \{ .*, refetchInterval: pollUnlessLive\([\d_]+, live\) \}$/,
];

describe("feed-derived hooks poll only without ticks (#648)", () => {
  it.each(HOOKS)("%s %s", (file, hook) => {
    const fn = body(file, hook);
    expect(fn).toContain("const live = useRealtimeLive();");
    const intervals = fn
      .split("\n")
      .map((line) => line.trim())
      .filter((line) => line.includes("refetchInterval:"));
    expect(intervals.length, "one interval").toBe(1);
    const shapes = fn.includes("at == null") ? MODE_AWARE : LIVE_ONLY;
    expect(
      shapes.some((shape) => shape.test(intervals[0])),
      intervals[0],
    ).toBe(true);
  });
});
