import {useSyncExternalStore} from "react";
import type {QueryClient} from "@tanstack/react-query";

import {API_BASE} from "./api";
import {getDesktopToken} from "./desktop-token";

/**
 * Realtime nudges: the backend pushes a small `{topic}` over the websocket when operational data
 * changes; we invalidate the matching React Query keys so those surfaces refetch instantly instead of
 * waiting for their poll. Purely additive — if the socket never connects, polling still keeps
 * everything correct. Keep these topics in sync with `backend/src/realtime.rs` `topic`.
 */
/** The feed ticks once per upstream VATSIM publish (#648). */
const FEED_TICK = "feed.tick";

/**
 * Every query derived from the VATSIM feed, with the spacing it used to poll at. They refetch on
 * {@link FEED_TICK} and — while the socket is live — stop polling on their own timers (see
 * {@link pollUnlessLive}).
 *
 * A tick refetches a key only once its data is at least `minGapMs` old. The feed publishes about every
 * 15s, so the 15s queries refetch on every tick, and a query that polled every 30s or 60s refetches
 * on every second or fourth: never more often than it used to poll, and always right on a publish
 * rather than up to a full interval after one (#648 AC3).
 */
const FEED_KEYS: { key: string[]; minGapMs: number }[] = [
  { key: ["flow-traffic"], minGapMs: 0 },
  { key: ["flow-atc"], minGapMs: 0 },
  { key: ["taxi"], minGapMs: 0 },
  { key: ["fca-counts"], minGapMs: 0 },
  { key: ["flow"], minGapMs: 20_000 },
  { key: ["aadc"], minGapMs: 20_000 },
  { key: ["fca-traffic"], minGapMs: 30_000 },
  { key: ["feed-status"], minGapMs: 30_000 },
  { key: ["idst"], minGapMs: 30_000 },
  { key: ["departures"], minGapMs: 60_000 },
  // Six hours of projection per ARTCC, computed once per feed snapshot on the server and shared by
  // every viewer (`AppState::sector_demand`): a miss costs one projection, 30–125 ms of CPU per ARTCC
  // in release; a hit costs a copy of the rows. Its cells are quarter-hour peaks, so once a minute.
  { key: ["sector-demand"], minGapMs: 60_000 },
];

export const TOPIC_KEYS: Record<string, string[][]> = {
  [FEED_TICK]: FEED_KEYS.map(({ key }) => key),
  // A release, CFR or GDP slot also moves the sector demand's proposed counts: see COALESCED_KEYS.
  "flow.release": [["idst"], ["fca-traffic"], ["departures"]],
  // Enabling, disabling or deleting an FCA changes which releases hold a wheels-up (#721).
  "flow.fca": [["fcas"], ["fca-traffic"], ["fca-counts"], ["idst"], ["event-fcas"]],
  "tmu.gdp": [["gdps"], ["gdp-board"], ["departures"]],
  "tmu.tmi": [["tmis"]],
  "tmu.groundstop": [["ground-stops"], ["departures"]],
  "tmu.program": [["tmu-programs"], ["departures"], ["flow"]],
  "tmu.advisory": [["advisories"]],
  "flow.cfr": [["departures"], ["flow"]],
  "events.availability": [["event-availability"]],
  // Payload-free by design: each client refetches its own data and works out whether the change
  // was about them. The socket is broadcast to every signed-in client, so it must not carry who.
  "access.granted": [["me"]],
  "events.reminder": [["my-ace-claims"]],
  "events.ace": [["event-ace"], ["my-ace-claims"]],
  "flow.runway": [["runway"], ["runway-configs"]],
  // A limit recolours the demand cells; a consolidation merges or splits its rows (#725). Both are a
  // deliberate edit someone is waiting to see, and rare, so they refetch at once. A limit-only change
  // re-judges the server's cached rows without re-projecting.
  "flow.sector_limits": [["sector-demand"]],
  "flow.sector_consolidations": [["sector-demand"]],
};

/**
 * Keys a topic refetches on the next {@link FEED_TICK} rather than at once (#725). A release, an FCA
 * switched on or off, a CFR or a GDP slot moves a wheels-up, so the sector demand's proposed counts
 * move, but these topics can arrive several times a minute while a program is running, and every
 * open client hears each one. Refetched at once, each would be a fresh projection on the server per
 * open ARTCC (its cache is keyed on the wheels-up too), multiplied by every burst.
 *
 * Held until the next tick instead, a burst collapses into one refetch, and every client makes it
 * together against the new snapshot, which the server projects once per ARTCC for all of them. So
 * these topics cost at most one projection per open ARTCC per feed publish, however many arrive and
 * however many clients hear them, and the change shows within one publish (about 15 s) instead of at
 * the key's next one-minute tick refetch. If no tick comes, {@link COALESCE_MS} refetches it anyway.
 */
export const COALESCED_KEYS: Record<string, string[][]> = {
  "flow.release": [["sector-demand"]],
  "flow.fca": [["sector-demand"]],
  "tmu.gdp": [["sector-demand"]],
  "flow.cfr": [["sector-demand"]],
};

/**
 * The longest a coalesced refetch waits for a tick: a little over one feed publish (~15 s), so a
 * tick normally gets there first, and at most one refetch per key per window when ticks are not
 * arriving.
 */
export const COALESCE_MS = 20_000;

/**
 * How often a query the socket nudges polls anyway (#649). The socket is the fast path; this is what
 * keeps "degrades cleanly to polling" true when a socket is down, a nudge is missed while a backend
 * replica's listener reconnects, or a key has no other refresh. A minute, since the nudge normally
 * gets there first.
 */
export const SOCKET_FALLBACK_MS = 60_000;

/** The subprotocol the server selects for a desktop client; the token travels beside it. */
const WS_PROTOCOL = "ois.v1";
const WS_BEARER_PREFIX = "ois.bearer.";

/** Every distinct key across all topics — refetched once on (re)connect to catch up on anything that
 *  changed while the socket was down. */
const ALL_KEYS: string[][] = [
  ...new Set(
    [...Object.values(TOPIC_KEYS), ...Object.values(COALESCED_KEYS)]
      .flat()
      .map((k) => JSON.stringify(k)),
  ),
].map((s) => JSON.parse(s) as string[]);

// ---- whether feed ticks are arriving (#648) ---------------------------------------------------------

let live = false;
const listeners = new Set<() => void>();

function setLive(next: boolean) {
  if (live === next) return;
  live = next;
  listeners.forEach((notify) => notify());
}

/**
 * How long without a {@link FEED_TICK} before the socket is treated as gone (#648 review): about twice
 * the feed's ~15 s cadence. A half-open socket — the network dropped and the browser hasn't noticed —
 * can stay "open" for minutes with nothing arriving; without this, every feed screen would stop polling
 * and freeze for that long. When the feed itself is quiet this only resumes polling, which is harmless.
 */
export const TICK_SILENCE_MS = 45_000;

/**
 * True while the realtime socket is open, the server has acknowledged a subscription that includes
 * {@link FEED_TICK}, **and** a tick has arrived within {@link TICK_SILENCE_MS}. False when signed out
 * (the socket only opens for signed-in users), while connecting, after a drop, and when ticks have gone
 * quiet — exactly the times a feed-derived screen must still poll.
 */
export function useRealtimeLive(): boolean {
  return useSyncExternalStore(
    (notify) => {
      listeners.add(notify);
      return () => listeners.delete(notify);
    },
    isRealtimeLive,
    () => false,
  );
}

/** The current value of {@link useRealtimeLive}, outside React. */
export function isRealtimeLive(): boolean {
  return live;
}

/**
 * A feed-derived query's `refetchInterval`: off while feed ticks are arriving, `ms` otherwise. The
 * ticks replace the timer rather than adding to it. One backend replica is assumed: with several, a
 * client would miss ticks from the others until the hub is shared (#649).
 */
export function pollUnlessLive(ms: number, isLive: boolean): number | false {
  return isLive ? false : ms;
}

function wsUrl(): string {
  // API_BASE is a full http(s) URL, or "" for a same-origin deployment.
  const base = API_BASE || (typeof window !== "undefined" ? window.location.origin : "");
  const url = new URL("/api/v1/ws", base);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  return url.toString();
}

/** One socket lifecycle event, kept for diagnostics reports (#629). */
export type RealtimeEvent = {at: string; event: "open" | "close" | "error" | "retry"; retry: number};

/** How many recent events {@link realtimeHistory} keeps. */
const HISTORY_EVENTS = 50;
const history: RealtimeEvent[] = [];

function record(event: RealtimeEvent["event"], retry: number) {
  history.push({at: new Date().toISOString(), event, retry});
  if (history.length > HISTORY_EVENTS) history.splice(0, history.length - HISTORY_EVENTS);
}

/** This window's recent realtime connection events, oldest first. */
export function realtimeHistory(): RealtimeEvent[] {
  return [...history];
}

/**
 * Connect the realtime socket and invalidate matching queries on each nudge. Auto-reconnects with
 * capped backoff. Returns a disposer that stops reconnecting and closes the socket.
 */
export function connectRealtime(qc: QueryClient): () => void {
  let ws: WebSocket | null = null;
  let retry = 0;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let closed = false;

  // Tick-silence watchdog (#648 review): live only while ticks keep arriving, not merely while the
  // socket claims to be open.
  // Coalesced refetches waiting for the next tick (#725), by key, and the timer that runs them if
  // no tick comes first.
  const pending = new Map<string, string[]>();
  let coalesce: ReturnType<typeof setTimeout> | null = null;
  const dropPending = () => {
    pending.clear();
    if (coalesce) clearTimeout(coalesce);
    coalesce = null;
  };
  /** Takes every pending key, clearing the fallback timer. */
  const takePending = () => {
    const keys = new Map(pending);
    dropPending();
    return keys;
  };
  const hold = (keys: string[][]) => {
    keys.forEach((key) => pending.set(JSON.stringify(key), key));
    coalesce ??= setTimeout(() => {
      coalesce = null;
      takePending().forEach((queryKey) => qc.invalidateQueries({ queryKey }));
    }, COALESCE_MS);
  };

  let tickSubscribed = false;
  let silence: ReturnType<typeof setTimeout> | null = null;
  const quiet = () => {
    if (silence) clearTimeout(silence);
    silence = null;
  };
  const heard = () => {
    quiet();
    setLive(true);
    silence = setTimeout(() => {
      silence = null;
      setLive(false);
    }, TICK_SILENCE_MS);
  };
  const notLive = () => {
    tickSubscribed = false;
    quiet();
    setLive(false);
  };

  // Subscribe narrowly (#589's Subscription, #648): every topic as before, plus the feed tick only
  // while some feed-derived query is on screen — so a page without one receives no ~15s ticks.
  const isFeedQuery = (key: readonly unknown[]) => FEED_KEYS.some(({ key: [prefix] }) => key[0] === prefix);
  const wantsFeed = () =>
    qc
      .getQueryCache()
      .getAll()
      .some((q) => isFeedQuery(q.queryKey) && q.getObserversCount() > 0);
  let sentFeed: boolean | null = null;
  const sendSubscription = () => {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    const feed = wantsFeed();
    if (feed === sentFeed) return;
    sentFeed = feed;
    const topics = Object.keys(TOPIC_KEYS).filter((t) => t !== FEED_TICK || feed);
    ws.send(JSON.stringify({ subscribe: topics }));
  };
  const stopWatchingCache = qc.getQueryCache().subscribe((event) => {
    if (event.type === "observerAdded" || event.type === "observerRemoved") sendSubscription();
  });

  const schedule = () => {
    if (closed || timer) return;
    const delay = Math.min(30_000, 1000 * 2 ** retry);
    retry += 1;
    record("retry", retry);
    timer = setTimeout(() => {
      timer = null;
      void open();
    }, delay);
  };

  const open = async () => {
    if (closed) return;

    // The desktop app has no `ois_session` cookie — sign-in runs in the system browser, so the
    // cookie is set there and never in the webview — and the WebSocket constructor cannot set an
    // Authorization header. The subprotocol list is the one request header it *can* set, so the
    // session token rides there. The server answers with the fixed `ois.v1` marker offered beside
    // it — a handshake only completes if one offered protocol is echoed, and echoing the token
    // itself would put the credential in the response too (`backend/src/realtime.rs`).
    // Without this the upgrade 401s and the desktop app gets no realtime nudges at all.
    let protocols: string[] | undefined;
    try {
      const token = await getDesktopToken();
      if (token) protocols = [WS_PROTOCOL, `${WS_BEARER_PREFIX}${token}`];
    } catch {
      /* no desktop token available; fall through to cookie auth */
    }
    if (closed) return;

    try {
      ws = protocols ? new WebSocket(wsUrl(), protocols) : new WebSocket(wsUrl());
    } catch {
      schedule();
      return;
    }
    ws.onopen = () => {
      retry = 0;
      sentFeed = null;
      sendSubscription();
      record("open", retry);
      // Catch up on anything that changed while we were (re)connecting; that covers anything held.
      dropPending();
      ALL_KEYS.forEach((queryKey) => qc.invalidateQueries({ queryKey }));
    };
    ws.onmessage = (e) => {
      try {
        const frame = JSON.parse(e.data as string) as {
          topic?: string;
          subscribed?: string[];
          error?: string;
        };
        // The server's answer to a subscribe frame: ticks are live only once it has accepted ours.
        // An error (an older server without `feed.tick`) changes nothing there, so keep polling.
        if (frame.subscribed) {
          if (frame.subscribed.includes(FEED_TICK)) {
            tickSubscribed = true;
            heard();
          } else {
            notLive();
          }
        }
        if (frame.error) notLive();
        if (frame.topic === FEED_TICK) {
          if (tickSubscribed) heard();
          const now = Date.now();
          // A held key refetches on this tick whatever its age; its gap holds only without one.
          const held = takePending();
          FEED_KEYS.forEach(({ key, minGapMs }) => {
            const forced = held.delete(JSON.stringify(key));
            qc.invalidateQueries({
              queryKey: key,
              predicate: (q) => forced || now - q.state.dataUpdatedAt >= minGapMs,
            });
          });
          held.forEach((queryKey) => qc.invalidateQueries({ queryKey }));
          return;
        }
        if (!frame.topic) return;
        TOPIC_KEYS[frame.topic]?.forEach((queryKey) => qc.invalidateQueries({ queryKey }));
        const held = COALESCED_KEYS[frame.topic];
        if (held) hold(held);
      } catch {
        /* ignore malformed frames */
      }
    };
    ws.onclose = () => {
      record("close", retry);
      ws = null;
      notLive();
      // The reconnect's catch-up refetches everything, so nothing held needs its own timer.
      dropPending();
      sentFeed = null;
      schedule();
    };
    ws.onerror = () => {
      record("error", retry);
      ws?.close();
    };
  };

  void open();
  return () => {
    closed = true;
    stopWatchingCache();
    notLive();
    dropPending();
    if (timer) clearTimeout(timer);
    ws?.close();
  };
}
