// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, beforeEach, describe, expect, it, vi} from "vitest";

type Q = {isSuccess: boolean; isPending: boolean; data: unknown[] | undefined};
const pending = (): Q => ({isSuccess: false, isPending: true, data: undefined});
const errored = (): Q => ({isSuccess: false, isPending: false, data: undefined});
const ok = (data: unknown[]): Q => ({isSuccess: true, isPending: false, data});

// The four restriction lists, each stepped through states the way a real poll moves them.
const lists = vi.hoisted(() => ({gs: {} as Q, gdp: {} as Q, tmi: {} as Q, prog: {} as Q}));
const notifyRestrictions = vi.hoisted(() => vi.fn());

vi.mock("@/lib/tmu", () => ({
  useGroundStops: () => lists.gs,
  useTmis: () => lists.tmi,
  usePrograms: () => lists.prog,
}));
vi.mock("@/lib/gdp", () => ({useGdps: () => lists.gdp}));
vi.mock("@/lib/auth", () => ({useMe: () => ({data: {server_admin: true}})}));
vi.mock("@/lib/historical-context", () => ({useHistoricalAt: () => null}));
vi.mock("@/lib/notify-restrictions", () => ({useRestrictionNotifier: () => notifyRestrictions}));

import {RestrictionAlerts} from "./restriction-alerts";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

let root: ReturnType<typeof createRoot> | undefined;
beforeEach(() => {
  lists.gs = pending();
  lists.gdp = pending();
  lists.tmi = pending();
  lists.prog = pending();
  notifyRestrictions.mockClear();
});
afterEach(() => {
  act(() => root?.unmount());
  root = undefined;
});

function mount() {
  root = createRoot(document.createElement("div"));
  const render = () => act(() => root!.render(<RestrictionAlerts />));
  render();
  return render;
}

const gs = (id: number, airport: string) => ({id, airport, status: "published", scope: "", until: null});
const announced = () => notifyRestrictions.mock.calls.flatMap(([alerts]) => alerts.map((a: {key: string}) => a.key));

// One row per list, each the minimum its alert builder reads.
const gdp = (id: number) => ({id, airport: "KEWR", status: "published", aar: 40, start_time: "1200", end_time: "1400", scope: ""});
const tmi = (id: number) => ({id, status: "published", requesting: "ZDC", providing: "ZNY", restriction: "20 MIT", decoded: ""});
const prog = (icao: string) => ({icao, mit: 20, aar: 0, jets_only: false});

describe("RestrictionAlerts (VATUSA/OIS#348 review)", () => {
  it("does not announce restrictions already in force when a list's first load failed", () => {
    const render = mount();
    lists.gdp = ok([]);
    lists.tmi = ok([]);
    lists.prog = ok([]);
    lists.gs = errored(); // e.g. the backend was mid-restart
    render();

    lists.gs = ok([gs(1, "KATL"), gs(2, "KJFK")]); // the next poll succeeds
    render();

    expect(notifyRestrictions).not.toHaveBeenCalled();
  });

  it("still alerts on the lists it can read when one needs a permission the user lacks", () => {
    const render = mount();
    lists.gs = ok([]);
    lists.tmi = ok([]);
    lists.prog = ok([]);
    lists.gdp = errored(); // 403: no tmu.gdp.read — permanently
    render();

    lists.gs = ok([gs(3, "KDCA")]);
    render();

    expect(announced()).toEqual(["gs:3"]);
  });

  it("announces a restriction that appears after its list first loaded, and only that one", () => {
    const render = mount();
    lists.gs = ok([gs(1, "KATL")]);
    lists.gdp = ok([]);
    lists.tmi = ok([]);
    lists.prog = ok([]);
    render();
    expect(notifyRestrictions).not.toHaveBeenCalled();

    lists.gs = ok([gs(1, "KATL"), gs(4, "KORD")]);
    render();

    expect(announced()).toEqual(["gs:4"]);
  });

  /**
   * Each list's silent seed is found by slicing its key's prefix and looking it up among the
   * loaded-list flags, so the key prefix and that flag's name are coupled by two bare strings that
   * nothing type-checks. Get them out of step for one list and that list inverts: every restriction
   * already in force announces itself on the first load, for ever. Only ground stops were covered,
   * so three of the four pairings were unpinned.
   */
  it.each([
    ["ground stops", "gs", () => (lists.gs = ok([gs(1, "KATL")])), () => (lists.gs = ok([gs(1, "KATL"), gs(2, "KJFK")])), "gs:2"],
    ["GDPs", "gdp", () => (lists.gdp = ok([gdp(1)])), () => (lists.gdp = ok([gdp(1), gdp(2)])), "gdp:2"],
    ["TMIs", "tmi", () => (lists.tmi = ok([tmi(1)])), () => (lists.tmi = ok([tmi(1), tmi(2)])), "tmi:2"],
    ["programs", "prog", () => (lists.prog = ok([prog("KATL")])), () => (lists.prog = ok([prog("KATL"), prog("KJFK")])), "prog:KJFK"],
  ])("seeds %s silently and then announces only what is new", (_label, _source, seed, arrive, expected) => {
    const render = mount();
    lists.gs = ok([]);
    lists.gdp = ok([]);
    lists.tmi = ok([]);
    lists.prog = ok([]);
    seed();
    render();
    expect(notifyRestrictions).not.toHaveBeenCalled();

    arrive();
    render();

    expect(announced()).toEqual([expected]);
  });
});
