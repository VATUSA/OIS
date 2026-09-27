// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {beforeAll, beforeEach, describe, expect, it, vi} from "vitest";

const notifyDesktop = vi.hoisted(() => vi.fn(async () => true));
const settings = vi.hoisted(() => ({values: {} as Record<string, unknown>}));

vi.mock("@/lib/desktop-notify", () => ({notifyDesktop}));
vi.mock("@/lib/settings", () => ({
  useSetting: (key: string, fallback: unknown) => ({value: key in settings.values ? settings.values[key] : fallback}),
}));

import {useRestrictionNotifier} from "./notify-restrictions";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});
beforeEach(() => notifyDesktop.mockClear());

function notifyWith(values: Record<string, unknown>) {
  settings.values = values;
  let notify: ReturnType<typeof useRestrictionNotifier> | undefined;
  function Probe() {
    notify = useRestrictionNotifier();
    return null;
  }
  const root = createRoot(document.createElement("div"));
  act(() => root.render(<Probe />));
  notify!([{key: "gs:1", kind: "Ground Stop", title: "KDCA", lines: ["Field-wide"]}]);
  act(() => root.unmount());
}

// Restrictions reach the sound through their own notifier, not the FCA detectors; nothing pinned
// that its sound follows its own toggle and volume (VATUSA/OIS#353 review).
describe("restriction sounds", () => {
  it("stays silent unless the restrictions sound is switched on", () => {
    notifyWith({"notifications.restrictions": true});
    expect(notifyDesktop).toHaveBeenCalledWith(expect.anything(), true, {enabled: false, volume: "normal"});
  });

  it("sounds at the chosen volume when switched on, banner or not", () => {
    notifyWith({"sounds.restrictions": true, "sounds.restrictions.volume": "loud"});
    expect(notifyDesktop).toHaveBeenCalledWith(expect.anything(), false, {enabled: true, volume: "loud"});
  });
});
