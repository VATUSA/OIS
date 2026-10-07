// @vitest-environment jsdom
import {QueryClient, QueryObserver} from "@tanstack/react-query";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";

const token = vi.hoisted(() => ({value: undefined as string | undefined}));
vi.mock("./desktop-token", () => ({getDesktopToken: async () => token.value}));
vi.mock("./api", () => ({API_BASE: "https://ois.example"}));

import {TICK_SILENCE_MS, connectRealtime, isRealtimeLive, pollUnlessLive} from "./realtime";

/** Records how each socket was opened, and lets a test push frames through it. */
class FakeSocket {
  static readonly OPEN = 1;
  static opened: FakeSocket[] = [];
  readyState = FakeSocket.OPEN;
  /** Frames the client sent — its subscribe frames (#648). */
  sent: string[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((e: {data: string}) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  constructor(
    readonly url: string,
    readonly protocols?: string[],
  ) {
    FakeSocket.opened.push(this);
  }
  send(frame: string) {
    this.sent.push(frame);
  }
  close() {}
}

const flush = () => new Promise((r) => setTimeout(r, 0));
const invalidated: unknown[] = [];
// A real client and cache — the subscription reads which queries are on screen from it.
let qc: QueryClient;

beforeEach(() => {
  FakeSocket.opened = [];
  invalidated.length = 0;
  qc = new QueryClient();
  vi.spyOn(qc, "invalidateQueries").mockImplementation((q) => {
    invalidated.push(q);
    return Promise.resolve();
  });
  vi.stubGlobal("WebSocket", FakeSocket);
});
afterEach(() => {
  vi.unstubAllGlobals();
  token.value = undefined;
});

describe("connectRealtime (VATUSA/OIS#348 review)", () => {
  it("authenticates a desktop socket by offering its token beside the ois.v1 marker", async () => {
    // The webview has no session cookie and a WebSocket can't send Authorization — without this
    // the upgrade 401s and the desktop app gets no realtime nudges at all.
    token.value = "ois_dsk_abc";
    const dispose = connectRealtime(qc);
    await flush();

    expect(FakeSocket.opened[0]?.url).toBe("wss://ois.example/api/v1/ws");
    expect(FakeSocket.opened[0]?.protocols).toEqual(["ois.v1", "ois.bearer.ois_dsk_abc"]);
    dispose();
  });

  it("offers no subprotocol on the web build, where the cookie authenticates", async () => {
    const dispose = connectRealtime(qc);
    await flush();

    expect(FakeSocket.opened[0]?.protocols).toBeUndefined();
    dispose();
  });

  it("refetches the user's own claims on an events.reminder nudge", async () => {
    const dispose = connectRealtime(qc);
    await flush();

    FakeSocket.opened[0]?.onmessage?.({data: JSON.stringify({topic: "events.reminder"})});
    expect(invalidated).toEqual([{queryKey: ["my-ace-claims"]}]);
    dispose();
  });

  // #647 worried that `"flow.fca"`'s `["event-fcas"]` matched no hook. It does: event FCAs are keyed
  // `fcaKey(eventId)` = `["event-fcas", eventId]`, and React Query invalidates by prefix by default, so
  // publishing or archiving an event FCA refreshes that event's own list. Pinned against a real client,
  // since a spy can't show which cached queries an invalidation actually reaches.
  it("refreshes an event's own FCA list on a flow.fca nudge", async () => {
    const real = new QueryClient();
    real.setQueryData(["event-fcas", 7], [{id: "fca"}]);
    real.setQueryData(["unrelated", 7], 1);
    const dispose = connectRealtime(real);
    await flush();
    // Connecting alone must not have done it, or this test would pass without the nudge.
    expect(real.getQueryState(["event-fcas", 7])?.isInvalidated).toBe(false);

    FakeSocket.opened[0]?.onmessage?.({data: JSON.stringify({topic: "flow.fca"})});
    await flush();
    expect(real.getQueryState(["event-fcas", 7])?.isInvalidated).toBe(true);
    expect(real.getQueryState(["unrelated", 7])?.isInvalidated).toBe(false);
    dispose();
  });
  // #643: a published or cancelled advisory refreshed only the publishing client's list.
  it("refetches the advisory list on a tmu.advisory nudge", async () => {
    const dispose = connectRealtime(qc);
    await flush();

    FakeSocket.opened[0]?.onmessage?.({data: JSON.stringify({topic: "tmu.advisory"})});
    expect(invalidated).toEqual([{queryKey: ["advisories"]}]);
    dispose();
  });
});

describe("ACE and runway topics (VATUSA/OIS#645, #646)", () => {
  async function nudge(topic: string) {
    const dispose = connectRealtime(qc);
    await flush();
    const socket = FakeSocket.opened.at(-1)!;
    socket.onopen?.();
    invalidated.length = 0; // drop the reconnect catch-up
    socket.onmessage?.({ data: JSON.stringify({ topic }) });
    dispose();
    return [...invalidated];
  }

  it("an ACE change refreshes the board and the user's own claims", async () => {
    expect(await nudge("events.ace")).toEqual([{ queryKey: ["event-ace"] }, { queryKey: ["my-ace-claims"] }]);
  });

  it("a runway change refreshes the runway view and the saved configurations", async () => {
    expect(await nudge("flow.runway")).toEqual([{ queryKey: ["runway"] }, { queryKey: ["runway-configs"] }]);
  });
});

// ==== VATUSA/OIS#648: the feed tick ==================================================================

const FEED = [
  ["flow-traffic"],
  ["flow-atc"],
  ["taxi"],
  ["fca-counts"],
  ["flow"],
  ["aadc"],
  ["fca-traffic"],
  ["feed-status"],
  ["idst"],
  ["departures"],
  ["sector-demand"],
];

/** Open the socket the client created and return it. */
async function opened() {
  await flush();
  const socket = FakeSocket.opened.at(-1)!;
  socket.onopen?.();
  return socket;
}

const subscribed = (socket: FakeSocket) =>
  socket.sent.map((f) => (JSON.parse(f) as { subscribe: string[] }).subscribe);

/** Put a feed-derived query on screen, the way a hook does — an observer on the real cache. */
function mount(queryKey: string[]) {
  const observer = new QueryObserver(qc, { queryKey, queryFn: async () => null, enabled: false });
  return observer.subscribe(() => {});
}

describe("the feed tick (VATUSA/OIS#648)", () => {
  it("refetches exactly the feed-derived queries", async () => {
    const dispose = connectRealtime(qc);
    const socket = await opened();
    invalidated.length = 0; // drop the reconnect catch-up

    socket.onmessage?.({ data: JSON.stringify({ topic: "feed.tick" }) });

    expect(invalidated.map((q) => (q as { queryKey: string[] }).queryKey)).toEqual(FEED);
    dispose();
  });

  // AC3 holds only if no feed key refetches more often than it polled before #648. These intervals are
  // each hook's `refetchInterval` on `next` before the tick existed, written out rather than read from
  // FEED_KEYS, so a changed spacing fails here instead of passing against itself.
  it.each([
    ["flow-traffic", 15_000],
    ["flow-atc", 15_000],
    ["taxi", 15_000],
    ["fca-counts", 15_000],
    ["flow", 20_000],
    ["aadc", 20_000],
    ["fca-traffic", 30_000],
    ["feed-status", 30_000],
    ["idst", 30_000],
    ["departures", 60_000],
    // New with #725, not a pre-#648 poll: its hook's fallback poll is the same minute.
    ["sector-demand", 60_000],
  ] as const)("a tick refetches %s no more often than its old %ims poll", async (prefix, polledMs) => {
    const dispose = connectRealtime(qc);
    const socket = await opened();
    invalidated.length = 0;
    // Freeze the clock across the tick: the handler reads `Date.now()` when the tick arrives and this
    // test reads it again, so an unfrozen millisecond between them flips the exact-boundary checks.
    vi.useFakeTimers({ toFake: ["Date"] });
    try {
      socket.onmessage?.({ data: JSON.stringify({ topic: "feed.tick" }) });

      const now = Date.now();
      const call = invalidated.find((q) => (q as { queryKey: string[] }).queryKey[0] === prefix) as
        | { predicate: (q: { state: { dataUpdatedAt: number } }) => boolean }
        | undefined;
      expect(call, `${prefix} refetches on a tick`).toBeDefined();
      const refetchesAt = (ageMs: number) => call!.predicate({ state: { dataUpdatedAt: now - ageMs } });
      if (polledMs <= 15_000) {
        // The feed publishes about every 15s, so a 15s query keeps up by refetching on every tick.
        expect(refetchesAt(1_000), `${prefix} refetches on every tick`).toBe(true);
      } else {
        expect(refetchesAt(polledMs - 1), `${prefix} waits out its old interval`).toBe(false);
        expect(refetchesAt(polledMs), `${prefix} refetches once its old interval has passed`).toBe(true);
      }
    } finally {
      vi.useRealTimers();
    }
    dispose();
  });

  it("refetches a slower query only once its old poll interval has passed", async () => {
    const dispose = connectRealtime(qc);
    const socket = await opened();
    invalidated.length = 0;
    socket.onmessage?.({ data: JSON.stringify({ topic: "feed.tick" }) });

    const now = Date.now();
    const gate = (prefix: string, ageMs: number) => {
      const call = invalidated.find((q) => (q as { queryKey: string[] }).queryKey[0] === prefix) as {
        predicate: (q: { state: { dataUpdatedAt: number } }) => boolean;
      };
      return call.predicate({ state: { dataUpdatedAt: now - ageMs } });
    };
    expect(gate("flow-traffic", 1_000), "a 15s query refetches on every tick").toBe(true);
    expect(gate("departures", 45_000), "a 60s query waits out its interval").toBe(false);
    expect(gate("departures", 61_000)).toBe(true);
    expect(gate("idst", 16_000)).toBe(false);
    expect(gate("idst", 31_000)).toBe(true);
    dispose();
  });

  it("subscribes to the tick only while a feed-derived query is on screen", async () => {
    const dispose = connectRealtime(qc);
    const socket = await opened();
    expect(subscribed(socket).at(-1)).not.toContain("feed.tick");
    expect(subscribed(socket).at(-1)).toContain("flow.release"); // everything else, as before

    const unmount = mount(["flow-traffic"]);
    expect(subscribed(socket).at(-1)).toContain("feed.tick");

    const sends = socket.sent.length;
    const unmountSecond = mount(["idst", "ZDC"]); // already subscribed: no new frame
    expect(socket.sent.length).toBe(sends);

    unmount();
    expect(socket.sent.length).toBe(sends); // still one feed query on screen
    unmountSecond();
    expect(subscribed(socket).at(-1)).not.toContain("feed.tick");
    dispose();
  });

  it("is live only once the server accepts a subscription with the tick, and not after a drop", async () => {
    const dispose = connectRealtime(qc);
    const socket = await opened();
    expect(isRealtimeLive()).toBe(false); // open, but not yet acknowledged

    socket.onmessage?.({ data: JSON.stringify({ subscribed: ["feed.tick", "flow.release"] }) });
    expect(isRealtimeLive()).toBe(true);

    socket.onmessage?.({ data: JSON.stringify({ subscribed: ["flow.release"] }) });
    expect(isRealtimeLive()).toBe(false);

    socket.onmessage?.({ data: JSON.stringify({ subscribed: ["feed.tick"] }) });
    socket.onmessage?.({ data: JSON.stringify({ error: "unknown_topic", topics: ["feed.tick"] }) });
    expect(isRealtimeLive()).toBe(false); // an older server: keep polling

    socket.onmessage?.({ data: JSON.stringify({ subscribed: ["feed.tick"] }) });
    socket.onclose?.();
    expect(isRealtimeLive()).toBe(false); // dropped: polling resumes
    dispose();
  });

  // #648 review: a half-open socket stays "open" with nothing arriving. Liveness must follow the ticks,
  // or every feed screen stops polling and freezes until the browser finally notices the drop.
  it("stops being live when ticks go quiet, and is live again on the next tick", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
    try {
      const dispose = connectRealtime(qc);
      await vi.advanceTimersByTimeAsync(0);
      const socket = FakeSocket.opened.at(-1)!;
      socket.onmessage?.({ data: JSON.stringify({ subscribed: ["feed.tick"] }) });
      expect(isRealtimeLive()).toBe(true);

      // Ticks keep it live, however long the session runs.
      for (let i = 0; i < 4; i++) {
        await vi.advanceTimersByTimeAsync(TICK_SILENCE_MS - 1_000);
        socket.onmessage?.({ data: JSON.stringify({ topic: "feed.tick" }) });
        expect(isRealtimeLive()).toBe(true);
      }

      // Silence past the window: not live, so the feed hooks poll again — the socket never closed.
      await vi.advanceTimersByTimeAsync(TICK_SILENCE_MS + 1_000);
      expect(isRealtimeLive()).toBe(false);

      // The next tick brings it back.
      socket.onmessage?.({ data: JSON.stringify({ topic: "feed.tick" }) });
      expect(isRealtimeLive()).toBe(true);
      dispose();
      expect(isRealtimeLive()).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });

  // A tick that arrives on a socket that isn't subscribed to them must not make it live.
  it("a stray tick without a tick subscription is not liveness", async () => {
    const dispose = connectRealtime(qc);
    const socket = await opened();
    socket.onmessage?.({ data: JSON.stringify({ subscribed: ["flow.release"] }) });
    socket.onmessage?.({ data: JSON.stringify({ topic: "feed.tick" }) });
    expect(isRealtimeLive()).toBe(false);
    dispose();
  });

  it("is never live once the socket is disposed (sign-out)", async () => {
    const dispose = connectRealtime(qc);
    const socket = await opened();
    socket.onmessage?.({ data: JSON.stringify({ subscribed: ["feed.tick"] }) });
    expect(isRealtimeLive()).toBe(true);
    dispose();
    expect(isRealtimeLive()).toBe(false);
  });

  it("polls only when ticks are not arriving", () => {
    expect(pollUnlessLive(15_000, true)).toBe(false);
    expect(pollUnlessLive(15_000, false)).toBe(15_000);
  });
});

/**
 * #648 AC3, measured on the real client over one simulated hour: a real `QueryObserver` (as a hook
 * mounts) counts every fetch, once polling as today and once driven by ticks through the real
 * socket handler and a real `invalidateQueries`. The feed publishes every 15s, 7s out of phase with
 * the client's own clock.
 */
describe("requests and latency over an hour (VATUSA/OIS#648 AC3)", () => {
  const HOUR = 3_600_000;
  const PUBLISH = 15_000;
  const PHASE = 7_000;

  beforeEach(() => {
    vi.mocked(qc.invalidateQueries).mockRestore(); // the real thing, so a tick really refetches
    vi.useFakeTimers({ toFake: ["setTimeout", "setInterval", "clearTimeout", "clearInterval", "Date"] });
  });
  afterEach(() => vi.useRealTimers());

  /** Mount `queryKey` with `interval`, run an hour (calling `onPublish` at each publish), and return
   *  when each fetch happened. */
  async function run(interval: number | false, onPublish: () => void, queryKey = ["flow-traffic"]) {
    const fetches: number[] = [];
    const observer = new QueryObserver(qc, {
      queryKey,
      queryFn: async () => {
        fetches.push(Date.now());
        return null;
      },
      refetchInterval: interval,
      staleTime: 0,
    });
    const unmount = observer.subscribe(() => {});
    const start = Date.now();
    for (let t = 0; t < HOUR; t += 1_000) {
      await vi.advanceTimersByTimeAsync(1_000);
      if ((Date.now() - start - PHASE) % PUBLISH === 0) onPublish();
    }
    unmount();
    return { fetches: fetches.map((f) => f - start), start };
  }

  const lagAfterPublishes = (fetches: number[]) => {
    const publishes = Array.from({ length: HOUR / PUBLISH }, (_, i) => i * PUBLISH + PHASE);
    return publishes
      .map((p) => fetches.find((f) => f >= p))
      .filter((f, i): f is number => f !== undefined && i < HOUR / PUBLISH - 1)
      .map((f, i) => f - (i * PUBLISH + PHASE));
  };

  it("tick-driven: no more requests than polling, and each update lands at the publish", async () => {
    const polling = await run(15_000, () => {});

    qc.clear();
    const dispose = connectRealtime(qc);
    await vi.advanceTimersByTimeAsync(0);
    const socket = FakeSocket.opened.at(-1)!;
    socket.onopen?.();
    socket.onmessage?.({ data: JSON.stringify({ subscribed: ["feed.tick"] }) });
    const ticking = await run(pollUnlessLive(15_000, isRealtimeLive()), () =>
      socket.onmessage?.({ data: JSON.stringify({ topic: "feed.tick" }) }),
    );
    dispose();

    // Requests per client-hour do not rise.
    expect(ticking.fetches.length).toBeLessThanOrEqual(polling.fetches.length);
    // Latency from publish to refetch: up to an interval under polling, immediate under the tick.
    expect(Math.max(...lagAfterPublishes(polling.fetches))).toBeGreaterThanOrEqual(5_000);
    expect(Math.max(...lagAfterPublishes(ticking.fetches))).toBe(0);
  });
});

describe("a slower query over an hour (VATUSA/OIS#648 AC3)", () => {
  beforeEach(() => {
    vi.mocked(qc.invalidateQueries).mockRestore();
    vi.useFakeTimers({ toFake: ["setTimeout", "setInterval", "clearTimeout", "clearInterval", "Date"] });
  });
  afterEach(() => vi.useRealTimers());

  it("departures (60s today) is not refetched more often than it polled", async () => {
    const HOUR = 3_600_000;
    const count = async (interval: number | false, onPublish: () => void) => {
      let fetches = 0;
      const observer = new QueryObserver(qc, {
        queryKey: ["departures", "KIAD"],
        queryFn: async () => {
          fetches += 1;
          return null;
        },
        refetchInterval: interval,
      });
      const unmount = observer.subscribe(() => {});
      const start = Date.now();
      for (let t = 0; t < HOUR; t += 1_000) {
        await vi.advanceTimersByTimeAsync(1_000);
        if ((Date.now() - start - 7_000) % 15_000 === 0) onPublish();
      }
      unmount();
      return fetches;
    };
    const polling = await count(60_000, () => {});

    qc.clear();
    const dispose = connectRealtime(qc);
    await vi.advanceTimersByTimeAsync(0);
    const socket = FakeSocket.opened.at(-1)!;
    socket.onopen?.();
    socket.onmessage?.({ data: JSON.stringify({ subscribed: ["feed.tick"] }) });
    const ticking = await count(pollUnlessLive(60_000, isRealtimeLive()), () =>
      socket.onmessage?.({ data: JSON.stringify({ topic: "feed.tick" }) }),
    );
    dispose();

    expect(ticking).toBeLessThanOrEqual(polling);
  });
});
