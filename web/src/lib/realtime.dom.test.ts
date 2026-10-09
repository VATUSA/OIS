// @vitest-environment jsdom
import {QueryClient, QueryObserver} from "@tanstack/react-query";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";

const token = vi.hoisted(() => ({value: undefined as string | undefined}));
vi.mock("./desktop-token", () => ({getDesktopToken: async () => token.value}));
vi.mock("./api", () => ({API_BASE: "https://ois.example"}));

import {COALESCE_MS, TICK_SILENCE_MS, connectRealtime, isRealtimeLive, pollUnlessLive} from "./realtime";

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

describe("sector demand topics (VATUSA/OIS#725)", () => {
  /** Connects, opens and subscribes to the tick, and drops the reconnect catch-up. */
  async function connected() {
    const dispose = connectRealtime(qc);
    await flush();
    const socket = FakeSocket.opened.at(-1)!;
    socket.onopen?.();
    socket.onmessage?.({ data: JSON.stringify({ subscribed: ["feed.tick"] }) });
    invalidated.length = 0;
    const send = (topic: string) => socket.onmessage?.({ data: JSON.stringify({ topic }) });
    return { dispose, send };
  }
  const demandCalls = () =>
    invalidated.filter((q) => (q as { queryKey: string[] }).queryKey[0] === "sector-demand") as {
      queryKey: string[];
      predicate?: (q: { state: { dataUpdatedAt: number } }) => boolean;
    }[];

  // #723 left the consolidation topic to this page: a merge or split must redraw the rows at once,
  // and a limit must recolour them. Both are a deliberate edit someone is waiting to see.
  it.each(["flow.sector_consolidations", "flow.sector_limits"])("%s refreshes every ARTCC's sector demand at once", async (topic) => {
    const { dispose, send } = await connected();
    send(topic);
    expect(invalidated).toEqual([{ queryKey: ["sector-demand"] }]);
    dispose();
  });

  // A moved wheels-up (release, CFR, GDP slot, an FCA switched off) moves the proposed counts, but the
  // refetch waits for the next tick, so a burst costs the server one projection per ARTCC, not one per
  // nudge per client. The topic's other keys still refetch at once.
  it.each([
    ["flow.release", [["idst"], ["fca-traffic"], ["departures"]]],
    ["flow.cfr", [["departures"], ["flow"]]],
    ["tmu.gdp", [["gdps"], ["gdp-board"], ["departures"]]],
    ["flow.fca", [["fcas"], ["fca-traffic"], ["fca-counts"], ["idst"], ["event-fcas"]]],
  ])("%s refreshes the sector demand on the next tick, whatever its age, and its other keys at once", async (topic, others) => {
    const { dispose, send } = await connected();
    send(topic);
    expect(invalidated).toEqual(others.map((queryKey) => ({ queryKey })));

    invalidated.length = 0;
    send("feed.tick");
    const [call, ...more] = demandCalls();
    expect(more).toEqual([]);
    // Fresh data (a second old) refetches on this tick, where the one-minute gap would skip it.
    expect(call.predicate!({ state: { dataUpdatedAt: Date.now() - 1_000 } })).toBe(true);

    // Held once: the tick after keeps the one-minute gap again.
    invalidated.length = 0;
    send("feed.tick");
    expect(demandCalls()[0].predicate!({ state: { dataUpdatedAt: Date.now() - 1_000 } })).toBe(false);
    dispose();
  });

  it("an unrelated topic leaves it alone, and the next tick keeps its gap", async () => {
    const { dispose, send } = await connected();
    send("tmu.advisory");
    expect(invalidated).toEqual([{ queryKey: ["advisories"] }]);
    invalidated.length = 0;
    send("feed.tick");
    expect(demandCalls()[0].predicate!({ state: { dataUpdatedAt: Date.now() - 1_000 } })).toBe(false);
    dispose();
  });

  describe("without a tick", () => {
    beforeEach(() => {
      vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
    });
    afterEach(() => vi.useRealTimers());

    async function connectedFake() {
      const dispose = connectRealtime(qc);
      await vi.advanceTimersByTimeAsync(0);
      const socket = FakeSocket.opened.at(-1)!;
      socket.onopen?.();
      invalidated.length = 0;
      const send = (topic: string) => socket.onmessage?.({ data: JSON.stringify({ topic }) });
      return { dispose, send, socket };
    }

    // Absolute times, not COALESCE_MS, so a changed window fails here instead of passing against itself:
    // past one feed publish (15 s), so a due tick gets there first, and within 20 s of the first nudge.
    it("refetches once, 20 s after the first nudge of a burst, and not before a tick was due", async () => {
      const { dispose, send } = await connectedFake();
      send("flow.release");
      await vi.advanceTimersByTimeAsync(5_000);
      send("flow.fca");
      send("tmu.gdp");
      await vi.advanceTimersByTimeAsync(10_000);
      expect(demandCalls(), "a tick due at 15 s gets there first").toEqual([]);
      await vi.advanceTimersByTimeAsync(4_999);
      expect(demandCalls()).toEqual([]);
      await vi.advanceTimersByTimeAsync(1);
      expect(demandCalls()).toEqual([{ queryKey: ["sector-demand"] }]);
      // Nothing else is held: no second refetch follows.
      await vi.advanceTimersByTimeAsync(COALESCE_MS * 3);
      expect(demandCalls()).toHaveLength(1);
      dispose();
    });

    it("a tick that comes first takes the refetch, and the timer adds none", async () => {
      const { dispose, send } = await connectedFake();
      send("flow.cfr");
      await vi.advanceTimersByTimeAsync(3_000);
      send("feed.tick");
      expect(demandCalls()).toHaveLength(1);
      await vi.advanceTimersByTimeAsync(COALESCE_MS * 2);
      expect(demandCalls()).toHaveLength(1);
      dispose();
    });

    it("drops what it held on a disconnect and on sign-out: the reconnect's catch-up covers it", async () => {
      const { send, socket } = await connectedFake();
      send("flow.release");
      socket.onclose?.();
      invalidated.length = 0;
      await vi.advanceTimersByTimeAsync(COALESCE_MS);
      expect(demandCalls()).toEqual([]);

      const again = await connectedFake();
      again.send("flow.release");
      again.dispose();
      await vi.advanceTimersByTimeAsync(COALESCE_MS * 2);
      expect(demandCalls()).toEqual([]);
    });
  });
});

describe("sector demand under a release burst (VATUSA/OIS#725)", () => {
  beforeEach(() => {
    vi.mocked(qc.invalidateQueries).mockRestore(); // the real thing, so a nudge really refetches
    vi.useFakeTimers({ toFake: ["setTimeout", "setInterval", "clearTimeout", "clearInterval", "Date"] });
  });
  afterEach(() => vi.useRealTimers());

  // An hour of a busy program: a release every 4 s and an FCA, GDP or CFR change every 20 s, over a
  // feed publishing every 15 s. One open ARTCC table, fetched each time its key refetches.
  it("refetches at most once per feed publish, and shows each change within one publish", async () => {
    const HOUR = 3_600_000;
    const dispose = connectRealtime(qc);
    await vi.advanceTimersByTimeAsync(0);
    const socket = FakeSocket.opened.at(-1)!;
    socket.onopen?.();
    socket.onmessage?.({ data: JSON.stringify({ subscribed: ["feed.tick"] }) });
    const send = (topic: string) => socket.onmessage?.({ data: JSON.stringify({ topic }) });

    const fetches: number[] = [];
    const observer = new QueryObserver(qc, {
      queryKey: ["sector-demand", "ZDC"],
      queryFn: async () => {
        fetches.push(Date.now());
        return null;
      },
      refetchInterval: pollUnlessLive(60_000, isRealtimeLive()),
      staleTime: 0,
    });
    const unmount = observer.subscribe(() => {});
    await vi.advanceTimersByTimeAsync(0);
    const start = Date.now();
    const nudges: number[] = [];
    const topics = ["flow.fca", "tmu.gdp", "flow.cfr"];
    for (let t = 1_000; t <= HOUR; t += 1_000) {
      await vi.advanceTimersByTimeAsync(1_000);
      if (t % 4_000 === 0) {
        send("flow.release");
        nudges.push(t);
      }
      if (t % 20_000 === 0) {
        send(topics[(t / 20_000) % 3]);
        nudges.push(t);
      }
      if (t % 15_000 === 7_000) send("feed.tick");
    }
    unmount();
    dispose();

    const after = fetches.map((f) => f - start).filter((f) => f > 0);
    // One per publish at most: 240 publishes an hour. Refetched at once, it would be one per nudge (1,080).
    expect(nudges.length).toBe(1_080);
    expect(after.length).toBeLessThanOrEqual(HOUR / 15_000);
    // Every nudge before the hour's last publish is followed by a fetch within one publish.
    const lastPublish = HOUR - ((HOUR - 7_000) % 15_000);
    const lag = nudges.filter((n) => n <= lastPublish).map((n) => after.find((f) => f >= n)! - n);
    expect(lag.length).toBeGreaterThan(1_000);
    expect(Math.max(...lag)).toBeLessThanOrEqual(15_000);
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
