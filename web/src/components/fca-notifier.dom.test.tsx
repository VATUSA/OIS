// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

// One controllable traffic query, stepped through states the way a real poll moves.
const traffic = vi.hoisted(() => ({
  current: {isSuccess: false, isPending: true, data: undefined as unknown},
}));
const notifyDesktop = vi.hoisted(() => vi.fn(async () => {}));

vi.mock("@/lib/fca", () => ({ useFcas: () => ({data: []}), useFcaTraffic: () => traffic.current }));
vi.mock("@/lib/settings", () => ({
  useSetting: (key: string, fallback: unknown) => ({value: key === "notifications.meteringDelayMin" ? "15" : key.startsWith("notifications.") ? true : fallback}),
}));
vi.mock("@/lib/desktop-notify", () => ({ notifyDesktop }));

import {FcaNotifier} from "./desktop-notifiers";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

const roots: ReturnType<typeof createRoot>[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) act(() => root.unmount());
  notifyDesktop.mockClear();
});

function mount() {
  const root = createRoot(document.createElement("div"));
  roots.push(root);
  const render = () => act(() => root.render(<FcaNotifier fcaId="f1" name="ZDC FCA" />));
  render();
  return render;
}

const flight = (callsign: string, edct: string) => ({callsign, edct, delay_min: 0});

describe("FcaNotifier (VATUSA/OIS#348 review)", () => {
  // An errored query is not pending, so "not pending" seeded the seen-set from no data — and the first
  // poll that succeeded then announced every flight already holding an EDCT, all at once.
  it("does not announce existing releases when the first load failed", () => {
    const render = mount();
    traffic.current = {isSuccess: false, isPending: false, data: undefined}; // errored
    render();
    traffic.current = {isSuccess: true, isPending: false, data: [flight("AAL1", "1200"), flight("UAL2", "1205")]};
    render();
    expect(notifyDesktop).not.toHaveBeenCalled();
  });

  it("announces a release that appears after the first successful load", () => {
    const render = mount();
    traffic.current = {isSuccess: true, isPending: false, data: [flight("AAL1", "1200")]};
    render();
    traffic.current = {isSuccess: true, isPending: false, data: [flight("AAL1", "1200"), flight("DAL3", "1210")]};
    render();
    expect(notifyDesktop).toHaveBeenCalledTimes(1);
    expect(notifyDesktop).toHaveBeenCalledWith(expect.objectContaining({title: "Release: DAL3"}), true);
  });
});
