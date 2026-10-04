// @vitest-environment jsdom
import type {QueryClient} from "@tanstack/react-query";
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
