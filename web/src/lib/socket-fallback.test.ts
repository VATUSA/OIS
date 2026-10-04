import {beforeEach, describe, expect, it, vi} from "vitest";

/**
 * #649: every query the realtime socket is the only refresh for also polls, so a dropped socket — or a
 * nudge missed while a backend replica's listener reconnects — heals within `SOCKET_FALLBACK_MS`
 * instead of leaving the TMU board silently stale. Historical snapshots never change, so they don't.
 */

const at = vi.hoisted(() => ({value: null as number | null}));
const captured = vi.hoisted((): {options: Record<string, unknown> | undefined} => ({options: undefined}));

vi.mock("@tanstack/react-query", () => ({
  useQuery: (options: Record<string, unknown>) => {
    captured.options = options;
    return {};
  },
  useMutation: () => ({}),
  useQueries: () => [],
  useQueryClient: () => ({}),
  keepPreviousData: Symbol("keepPreviousData"),
}));
vi.mock("@ois/ui", () => ({useToast: () => ({})}));
vi.mock("./historical-context", () => ({useHistoricalAt: () => at.value}));
vi.mock("./api", () => ({ois: {}, API_BASE: ""}));

import {useEventAvailability} from "./availability";
import {useFcas} from "./fca";
import {useGdps} from "./gdp";
import {SOCKET_FALLBACK_MS} from "./realtime";
import {useGroundStops, usePrograms, useTmis} from "./tmu";

const pollOf = (hook: () => unknown) => {
  captured.options = undefined;
  hook();
  // Read back through a cast: TS narrows `options` to `undefined` above, unaware `hook` set it.
  const options = captured.options as Record<string, unknown> | undefined;
  return options?.refetchInterval;
};

beforeEach(() => {
  at.value = null;
});

describe("socket-only queries poll as a fallback", () => {
  it.each([
    ["TMIs", () => useTmis()],
    ["rate programs", () => usePrograms()],
    ["ground stops", () => useGroundStops()],
    ["GDPs", () => useGdps()],
    ["live FCAs", () => useFcas()],
    ["an event's FCAs", () => useFcas(7)],
    ["event availability", () => useEventAvailability(7)],
  ])("%s", (_, hook) => {
    expect(pollOf(hook)).toBe(SOCKET_FALLBACK_MS);
  });

  it("but a historical snapshot does not poll", () => {
    at.value = 1_700_000_000;
    for (const hook of [() => useTmis(), () => useGroundStops(), () => useGdps(), () => useFcas()]) {
      expect(pollOf(hook)).toBe(false);
    }
  });
});
