import {beforeEach, describe, expect, it, vi} from "vitest";

// Captures the options `useMyAceClaims` hands to TanStack, as `socket-fallback.test.ts` does, so the
// query key, `enabled` and the queryFn are read from the hook itself rather than from a mount.
const captured = vi.hoisted((): {options: Record<string, unknown> | undefined} => ({options: undefined}));
const client = vi.hoisted(() => ({response: {} as {data?: unknown; error?: unknown}, paths: [] as string[]}));

vi.mock("@tanstack/react-query", () => ({
  useQuery: (options: Record<string, unknown>) => {
    captured.options = options;
    return {};
  },
  useMutation: () => ({}),
  useQueryClient: () => ({}),
}));
vi.mock("@ois/ui", () => ({useToast: () => ({})}));
vi.mock("./api", () => ({
  ois: {
    GET: async (path: string) => {
      client.paths.push(path);
      return client.response;
    },
  },
  API_BASE: "",
}));

import {useMyAceClaims} from "./ace";
import {SOCKET_FALLBACK_MS} from "./realtime";

const optionsOf = (hook: () => unknown) => {
  captured.options = undefined;
  hook();
  // Read back through a cast: TS narrows `options` to `undefined` above, unaware the hook set it.
  const options = captured.options as Record<string, unknown> | undefined;
  if (!options) throw new Error("useMyAceClaims did not call useQuery");
  return options;
};
const run = () => (optionsOf(() => useMyAceClaims(true)).queryFn as () => Promise<unknown>)();

const CLAIM = {
  claim_id: "c1",
  event_id: 9,
  event_title: "Fly-In",
  position: "DCA_APP",
  start_time: "2026-09-28T01:00:00Z",
};

beforeEach(() => {
  client.response = {};
  client.paths = [];
});

describe("useMyAceClaims (VATUSA/OIS#958)", () => {
  it("uses the query key the realtime topics invalidate", () => {
    expect(optionsOf(() => useMyAceClaims(true)).queryKey).toEqual(["my-ace-claims"]);
  });

  it("passes enabled through, false and true", () => {
    expect(optionsOf(() => useMyAceClaims(false)).enabled).toBe(false);
    expect(optionsOf(() => useMyAceClaims(true)).enabled).toBe(true);
  });

  it("polls at SOCKET_FALLBACK_MS", () => {
    expect(SOCKET_FALLBACK_MS).not.toBe(false);
    expect(optionsOf(() => useMyAceClaims(true)).refetchInterval).toBe(SOCKET_FALLBACK_MS);
  });

  it("throws when the client returns an error, instead of returning no claims", async () => {
    client.response = {data: undefined, error: {code: "unauthorized"}};
    await expect(run()).rejects.toThrow("failed to load your ACE claims");
    expect(client.paths).toEqual(["/api/v1/me/ace-claims"]);
  });

  // openapi-fetch answers a failed response with an empty body as `{error: undefined}`, so the
  // `!data` half is what catches it.
  it("throws when the client returns neither data nor an error (a failed response with no body)", async () => {
    client.response = {};
    await expect(run()).rejects.toThrow("failed to load your ACE claims");
  });

  it("returns the claims the client returned", async () => {
    client.response = {data: [CLAIM]};
    await expect(run()).resolves.toEqual([CLAIM]);
    expect(client.paths).toEqual(["/api/v1/me/ace-claims"]);
  });
});
