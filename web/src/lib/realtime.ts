import type {QueryClient} from "@tanstack/react-query";

import {API_BASE} from "./api";

/**
 * Realtime nudges: the backend pushes a small `{topic}` over the websocket when operational data
 * changes; we invalidate the matching React Query keys so those surfaces refetch instantly instead of
 * waiting for their poll. Purely additive — if the socket never connects, polling still keeps
 * everything correct. Keep these topics in sync with `backend/src/realtime.rs` `topic`.
 */
const TOPIC_KEYS: Record<string, string[][]> = {
  "flow.release": [["idst"], ["fca-traffic"], ["departures"]],
  "flow.fca": [["fcas"], ["fca-traffic"], ["fca-counts"], ["idst"], ["event-fcas"]],
  "tmu.gdp": [["gdps"], ["gdp-board"], ["departures"]],
  "tmu.tmi": [["tmis"]],
  "tmu.groundstop": [["ground-stops"], ["departures"]],
  "tmu.program": [["tmu-programs"], ["departures"], ["flow"]],
  "flow.cfr": [["departures"], ["flow"]],
  "events.availability": [["event-availability"]],
  // Payload-free by design: each client refetches its own data and works out whether the change
  // was about them. The socket is broadcast to every signed-in client, so it must not carry who.
  "access.granted": [["me"]],
  "events.reminder": [["ace-claims"], ["my-ace-claims"]],
};

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
    timer = setTimeout(() => {
      timer = null;
      open();
    }, delay);
  };

  const open = () => {
    if (closed) return;
    try {
      ws = new WebSocket(wsUrl());
    } catch {
      schedule();
      return;
    }
    ws.onopen = () => {
      retry = 0;
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
      ws = null;
      schedule();
    };
    ws.onerror = () => ws?.close();
  };

  open();
  return () => {
    closed = true;
    if (timer) clearTimeout(timer);
    ws?.close();
  };
}
