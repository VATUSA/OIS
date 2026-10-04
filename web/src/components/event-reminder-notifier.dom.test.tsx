// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {afterEach, beforeAll, beforeEach, describe, expect, it, vi} from "vitest";

const notifyDesktop = vi.hoisted(() => vi.fn(async (_n: {title: string}, _enabled: boolean) => true));
const claims = vi.hoisted(() => ({rows: [] as unknown[], calls: 0}));

vi.mock("@/lib/desktop-notify", () => ({notifyDesktop}));
// The claims query also polls as the socket's fallback (#649). These tests pin that a reminder fires
// on the clock alone — with no nudge and no refetch — so that poll is switched off here; it is pinned
// for the other socket-only queries in `lib/socket-fallback.test.ts`.
vi.mock("@/lib/realtime", () => ({SOCKET_FALLBACK_MS: false}));
vi.mock("@/lib/platform", () => ({can: () => true}));
vi.mock("@/lib/permissions", () => ({hasPermission: () => true}));
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
  const qc = new QueryClient();
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
