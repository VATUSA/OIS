// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {afterEach, beforeAll, beforeEach, describe, expect, it, vi} from "vitest";

const notifyDesktop = vi.hoisted(() => vi.fn(async (_n: {title: string}, _enabled: boolean) => true));
const claims = vi.hoisted(() => ({rows: [] as unknown[], calls: 0, fail: false}));
const perm = vi.hoisted(() => ({claim: true}));

vi.mock("@/lib/desktop-notify", () => ({notifyDesktop}));
// The claims query also polls as the socket's fallback (#649). These tests pin that a reminder fires
// on the clock alone — with no nudge and no refetch — so that poll is switched off here; it is pinned
// for the other socket-only queries in `lib/socket-fallback.test.ts`.
vi.mock("@/lib/realtime", () => ({SOCKET_FALLBACK_MS: false}));
vi.mock("@/lib/platform", () => ({can: () => true}));
vi.mock("@/lib/permissions", () => ({
  hasPermission: (_me: unknown, p: string) => p === "ace.requests.claim" && perm.claim,
}));
vi.mock("@/lib/auth", () => ({useMe: () => ({data: {id: "u1", role_names: [], permissions: {}}})}));
vi.mock("@/lib/settings", () => ({
  useSetting: (key: string, fallback: unknown) => ({
    value: key === "notifications.eventReminders" ? true : fallback,
  }),
}));
// A fresh copy per response, as the network gives — so identical claims are equal but not the same
// object, which is exactly what TanStack's structural sharing collapses back to the old reference.
vi.mock("@/lib/api", () => ({
  ois: {
    GET: async () => {
      claims.calls += 1;
      // openapi-fetch's shape for a refused or failed response: no data, an error body.
      if (claims.fail) return {data: undefined, error: {code: "internal"}};
      return {data: structuredClone(claims.rows)};
    },
  },
}));

import {EventReminderNotifier} from "./desktop-notifiers";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

const HOUR = 3_600_000;
const T0 = new Date("2026-09-27T00:00:00Z").getTime();
const settle = () => act(async () => { await vi.advanceTimersByTimeAsync(50); });

let root: ReturnType<typeof createRoot> | undefined;
beforeEach(() => {
  vi.useFakeTimers({now: T0});
  notifyDesktop.mockClear();
  claims.calls = 0;
  claims.fail = false;
  perm.claim = true;
  claims.rows = [
    {
      claim_id: "c1",
      event_id: 9,
      event_title: "Fly-In",
      position: "DCA_APP",
      start_time: new Date(T0 + 25 * HOUR).toISOString(),
    },
  ];
});
afterEach(() => {
  act(() => root?.unmount());
  root = undefined;
  vi.useRealTimers();
});

async function mount() {
  // No retries: the app's default three would hold a failing query in `fetching` past the assertions.
  const qc = new QueryClient({defaultOptions: {queries: {retry: false}}});
  root = createRoot(document.createElement("div"));
  await act(async () =>
    root!.render(
      <QueryClientProvider client={qc}>
        <EventReminderNotifier />
      </QueryClientProvider>,
    ),
  );
  await settle();
  return qc;
}

describe("EventReminderNotifier (VATUSA/OIS#348 review)", () => {
  it("reminds once a claim crosses T-24h and the events.reminder nudge refetches it", async () => {
    const qc = await mount();
    expect(notifyDesktop).not.toHaveBeenCalled(); // 25h out: nothing due yet

    // Jump the clock without firing the minute tick, so only the nudge can surface the reminder.
    vi.setSystemTime(T0 + 1.5 * HOUR);
    await act(async () => {
      await qc.invalidateQueries({queryKey: ["my-ace-claims"]}); // what realtime.ts does
    });
    await settle();

    expect(claims.calls).toBe(2);
    expect(notifyDesktop).toHaveBeenCalledTimes(1);
    expect(notifyDesktop).toHaveBeenCalledWith(
      expect.objectContaining({title: "Fly-In in 24 hours", route: "/planning/events/9"}),
      true,
      {enabled: false, volume: "normal"},
    );
  });

  it("reminds on the clock alone when no nudge arrives", async () => {
    await mount();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1.5 * HOUR);
    });

    expect(claims.calls).toBe(1); // no refetch: the tick did it
    expect(notifyDesktop).toHaveBeenCalledWith(
      expect.objectContaining({title: "Fly-In in 24 hours"}),
      true,
      {enabled: false, volume: "normal"},
    );
  });

  it("reminds at each tier once, not on every tick", async () => {
    await mount();

    // Hour by hour, each rendered — through T-24h (at +1h) and T-6h (at +19h).
    for (let h = 0; h < 20; h++) {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(HOUR);
      });
    }

    expect(notifyDesktop.mock.calls.map(([n]) => n.title)).toEqual([
      "Fly-In in 24 hours",
      "Fly-In in 6 hours",
    ]);
  });
});

const KEY = ["my-ace-claims"];
const advance = (ms: number) => act(async () => { await vi.advanceTimersByTimeAsync(ms); });
const refetch = async (qc: QueryClient) => {
  await act(async () => {
    await qc.invalidateQueries({queryKey: KEY});
  });
  await settle();
};
const titles = () => notifyDesktop.mock.calls.map(([n]) => n.title);

describe("EventReminderNotifier when the claims fetch fails (VATUSA/OIS#958)", () => {
  it("a failed first fetch leaves the query in error, not an empty success, and fires nothing", async () => {
    claims.fail = true;
    const qc = await mount();

    expect(claims.calls).toBe(1);
    expect(qc.getQueryState(KEY)?.status).toBe("error");
    expect(qc.getQueryData(KEY)).toBeUndefined();

    await advance(1.5 * HOUR); // c1 is inside T-24h now
    expect(notifyDesktop).not.toHaveBeenCalled();
  });

  it("recovering from a failed first fetch seeds silently instead of firing every due claim", async () => {
    claims.rows = [
      {claim_id: "c1", event_id: 9, event_title: "Fly-In", position: "DCA_APP",
        start_time: new Date(T0 + 23 * HOUR).toISOString()}, // inside T-24h from the start
    ];
    claims.fail = true;
    const qc = await mount();
    expect(qc.getQueryState(KEY)?.status).toBe("error");

    claims.fail = false;
    await refetch(qc);
    expect(qc.getQueryState(KEY)?.status).toBe("success");
    expect(notifyDesktop).not.toHaveBeenCalled();

    // Positive control: the seed is live, so a claim crossing a tier after it still fires.
    await advance(18 * HOUR); // c1 crosses T-6h
    expect(titles()).toEqual(["Fly-In in 6 hours"]);
  });

  it("a failed refetch after a success leaves the query in error with the last claims kept", async () => {
    const qc = await mount();
    expect(qc.getQueryState(KEY)?.status).toBe("success");

    claims.fail = true;
    await refetch(qc);

    expect(claims.calls).toBe(2);
    expect(qc.getQueryState(KEY)?.status).toBe("error");
    expect(qc.getQueryData(KEY)).toEqual([expect.objectContaining({claim_id: "c1"})]);
  });

  it("a claim crossing T-24h while the query is in error fires nothing, then fires once on recovery", async () => {
    const qc = await mount();
    claims.fail = true;
    await refetch(qc);

    await advance(1.5 * HOUR); // c1 crosses T-24h on the minute tick while the query is in error
    expect(notifyDesktop).not.toHaveBeenCalled();

    claims.fail = false;
    await refetch(qc);
    expect(titles()).toEqual(["Fly-In in 24 hours"]);
  });

  it("after recovery, a reminder sent before the error is not repeated and one due during it fires once", async () => {
    claims.rows = [
      {claim_id: "c1", event_id: 9, event_title: "Fly-In", position: "DCA_APP",
        start_time: new Date(T0 + 25 * HOUR).toISOString()},
      {claim_id: "c2", event_id: 10, event_title: "Overload", position: "PCT_APP",
        start_time: new Date(T0 + 27 * HOUR).toISOString()},
    ];
    const qc = await mount();

    await advance(1.5 * HOUR); // c1 crosses T-24h with the query healthy
    expect(titles()).toEqual(["Fly-In in 24 hours"]);

    claims.fail = true;
    await refetch(qc);
    await advance(2 * HOUR); // c2 crosses T-24h while the query is in error
    expect(titles()).toEqual(["Fly-In in 24 hours"]);

    claims.fail = false;
    await refetch(qc);
    await advance(HOUR);
    expect(titles()).toEqual(["Fly-In in 24 hours", "Overload in 24 hours"]);
  });

  it("without ace.requests.claim the claims query never runs and nothing fires", async () => {
    perm.claim = false;
    const qc = await mount();
    await advance(1.5 * HOUR);

    expect(claims.calls).toBe(0);
    expect(qc.getQueryState(KEY)?.status).toBe("pending");
    expect(notifyDesktop).not.toHaveBeenCalled();
  });
});
