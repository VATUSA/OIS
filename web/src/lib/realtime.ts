import type {QueryClient} from "@tanstack/react-query";

import {API_BASE} from "./api";
import {getDesktopToken} from "./desktop-token";

/**
 * Realtime nudges: the backend pushes a small `{topic}` over the websocket when operational data
 * changes; we invalidate the matching React Query keys so those surfaces refetch instantly instead of
 * waiting for their poll. Purely additive — if the socket never connects, polling still keeps
 * everything correct. Keep these topics in sync with `backend/src/realtime.rs` `topic`.
 */
export const TOPIC_KEYS: Record<string, string[][]> = {
  "flow.release": [["idst"], ["fca-traffic"], ["departures"]],
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
};

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
    Object.values(TOPIC_KEYS)
      .flat()
      .map((k) => JSON.stringify(k)),
  ),
].map((s) => JSON.parse(s) as string[]);

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
      record("open", retry);
      // Catch up on anything that changed while we were (re)connecting.
      ALL_KEYS.forEach((queryKey) => qc.invalidateQueries({ queryKey }));
    };
    ws.onmessage = (e) => {
      try {
        const { topic } = JSON.parse(e.data as string) as { topic?: string };
        (topic ? TOPIC_KEYS[topic] : undefined)?.forEach((queryKey) =>
          qc.invalidateQueries({ queryKey }),
        );
      } catch {
        /* ignore malformed frames */
      }
    };
    ws.onclose = () => {
      record("close", retry);
      ws = null;
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
    if (timer) clearTimeout(timer);
    ws?.close();
  };
}
