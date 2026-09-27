// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, beforeEach, describe, expect, it, vi} from "vitest";

const state = vi.hoisted(() => ({
  me: undefined as unknown,
  fcas: [] as {id: string; name: string; artcc: string; enabled: boolean}[],
}));
const trafficCalls = vi.hoisted(() => [] as unknown[][]);
const notifyDesktop = vi.hoisted(() => vi.fn(async () => true));

vi.mock("@/lib/auth", () => ({useMe: () => ({data: state.me})}));
vi.mock("@/lib/fca", () => ({
  useFcas: () => ({data: state.fcas}),
  useFcaTraffic: (...args: unknown[]) => {
    trafficCalls.push(args);
    return {isSuccess: false, data: undefined};
  },
}));
vi.mock("@/lib/settings", () => ({
  useSetting: (key: string, fallback: unknown) => ({
    value: key === "notifications.meteringDelayMin" ? "15" : key.startsWith("notifications.") ? true : fallback,
  }),
}));
vi.mock("@/lib/desktop-notify", () => ({notifyDesktop}));

import {AccessNotifier, FcaNotifiers} from "./desktop-notifiers";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

let root: ReturnType<typeof createRoot> | undefined;
beforeEach(() => {
  trafficCalls.length = 0;
  notifyDesktop.mockClear();
  state.fcas = [
    {id: "zdc-1", name: "ZDC FCA", artcc: "ZDC", enabled: true},
    {id: "zny-1", name: "ZNY FCA", artcc: "ZNY", enabled: true},
    {id: "zob-1", name: "ZOB FCA", artcc: "ZOB", enabled: true},
    {id: "zdc-off", name: "ZDC off", artcc: "ZDC", enabled: false},
  ];
});
afterEach(() => {
  act(() => root?.unmount());
  root = undefined;
});

function render(node: React.ReactNode) {
  root ??= createRoot(document.createElement("div"));
  act(() => root!.render(node));
}

const me = (over: Record<string, unknown> = {}) => ({
  server_admin: false,
  role_names: [],
  permissions: {},
  vatusa: {home_facility: "ZDC", visits: ["ZNY"]},
  ...over,
});
const polled = () => trafficCalls.map(([id]) => id).sort();

describe("FcaNotifiers scope (VATUSA/OIS#348 review)", () => {
  it("polls only the enabled FCAs of the user's home and visiting ARTCCs", () => {
    state.me = me();
    render(<FcaNotifiers />);
    expect(polled()).toEqual(["zdc-1", "zny-1"]);
  });

  it("asks for polling that keeps running while the window is hidden", () => {
    state.me = me();
    render(<FcaNotifiers />);
    // `refetchInterval` is skipped for a hidden document unless this is set — and hidden is when
    // these notifications matter.
    expect(trafficCalls[0]).toEqual(["zdc-1", false, {background: true}]);
  });

  it("covers every ARTCC for a server admin", () => {
    state.me = me({server_admin: true, vatusa: null});
    render(<FcaNotifiers />);
    expect(polled()).toEqual(["zdc-1", "zny-1", "zob-1"]);
  });

  it("covers nothing until the VATUSA profile says where the user is", () => {
    state.me = me({vatusa: null});
    render(<FcaNotifiers />);
    expect(polled()).toEqual([]);
  });
});

describe("AccessNotifier (VATUSA/OIS#348 review)", () => {
  it("raises one notification for a role grant, not one per permission it brings", () => {
    state.me = me({role_names: ["USER"], permissions: {auth: {profile: ["read"]}}});
    render(<AccessNotifier />);

    state.me = me({
      role_names: ["USER", "TMU"],
      permissions: {auth: {profile: ["read"]}, tmu: {program: ["read", "update"], gdp: ["read", "update"]}},
    });
    render(<AccessNotifier />);

    expect(notifyDesktop).toHaveBeenCalledTimes(1);
    expect(notifyDesktop).toHaveBeenCalledWith(
      {category: "access", title: "Access granted", body: "You were given the TMU role.", route: "/profile"},
      true,
    );
  });

  it("names a single permission, and says nothing for a revoke", () => {
    state.me = me({permissions: {flow: {fca: ["read"]}}});
    render(<AccessNotifier />);

    state.me = me({permissions: {flow: {fca: ["read", "update"]}}});
    render(<AccessNotifier />);
    state.me = me({permissions: {flow: {fca: ["read"]}}});
    render(<AccessNotifier />);

    expect(notifyDesktop).toHaveBeenCalledTimes(1);
    expect(notifyDesktop).toHaveBeenCalledWith(
      expect.objectContaining({body: "You were given flow.fca.update."}),
      true,
    );
  });

  it("counts several permissions granted together", () => {
    state.me = me({permissions: {}});
    render(<AccessNotifier />);

    state.me = me({permissions: {flow: {fca: ["read", "update"]}, tmu: {tmi: ["read"]}}});
    render(<AccessNotifier />);

    expect(notifyDesktop).toHaveBeenCalledTimes(1);
    expect(notifyDesktop).toHaveBeenCalledWith(
      expect.objectContaining({body: "You were given 3 new permissions."}),
      true,
    );
  });
});
