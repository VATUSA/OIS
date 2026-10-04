// @vitest-environment jsdom
import {QueryClient} from "@tanstack/react-query";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";

const token = vi.hoisted(() => ({value: undefined as string | undefined}));
vi.mock("./desktop-token", () => ({getDesktopToken: async () => token.value}));
vi.mock("./api", () => ({API_BASE: "https://ois.example"}));

import {connectRealtime} from "./realtime";

/** Records how each socket was opened, and lets a test push frames through it. */
class FakeSocket {
  static opened: FakeSocket[] = [];
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
  close() {}
}

const flush = () => new Promise((r) => setTimeout(r, 0));
const invalidated: unknown[] = [];
const qc = {invalidateQueries: (q: unknown) => invalidated.push(q)} as unknown as QueryClient;

beforeEach(() => {
  FakeSocket.opened = [];
  invalidated.length = 0;
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
});
