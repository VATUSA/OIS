// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, beforeEach, describe, expect, it, vi} from "vitest";

/**
 * That the restriction notifier hands on **its own** sound setting (#353).
 *
 * `sounds.restrictions` is a separate switch from `notifications.restrictions`, and passing the
 * wrong one — or a literal `true` — went unnoticed: nothing exercised this hook, so the sound could
 * have fired for everyone with restriction *banners* on, or for everyone full stop.
 */
const settings = vi.hoisted(() => ({} as Record<string, unknown>));
vi.mock("@/lib/settings", () => ({
  useSetting: <T,>(key: string, fallback: T) => ({
    value: (settings[key] as T | undefined) ?? fallback,
  }),
}));
type Sound = {enabled: boolean; volume?: string} | undefined;
// Typed parameters so the assertions can read the third argument off `mock.calls`.
const notifyDesktop = vi.hoisted(() =>
  vi.fn((_notification: unknown, _enabled: boolean, _sound?: Sound) => Promise.resolve(true)),
);
vi.mock("@/lib/desktop-notify", () => ({notifyDesktop}));

import {useRestrictionNotifier} from "./notify-restrictions";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

let root: ReturnType<typeof createRoot> | undefined;
afterEach(() => {
  act(() => root?.unmount());
  root = undefined;
});
beforeEach(() => {
  notifyDesktop.mockClear();
  for (const key of Object.keys(settings)) delete settings[key];
});

const ALERT = {key: "gs:1", kind: "Ground Stop", title: "KATL", lines: ["Field-wide"]};

/** Mounts the hook and fires one alert through it. */
function announce() {
  function Harness() {
    const notify = useRestrictionNotifier();
    notify([ALERT]);
    return null;
  }
  root = createRoot(document.createElement("div"));
  act(() => root!.render(<Harness />));
  return notifyDesktop.mock.calls[0];
}

describe("useRestrictionNotifier sound settings (VATUSA/OIS#353 review)", () => {
  it("asks for no sound when the sound toggle is off, even with banners on", () => {
    settings["notifications.restrictions"] = true;
    const call = announce();
    expect(call?.[1]).toBe(true);
    expect(call?.[2]).toEqual({enabled: false, volume: "normal"});
  });

  it("asks for a sound when its own toggle is on, even with banners off", () => {
    settings["sounds.restrictions"] = true;
    const call = announce();
    expect(call?.[1]).toBe(false);
    expect(call?.[2]).toEqual({enabled: true, volume: "normal"});
  });

  it("passes the category's own volume", () => {
    settings["sounds.restrictions"] = true;
    settings["sounds.restrictions.volume"] = "loud";
    expect(announce()?.[2]).toEqual({enabled: true, volume: "loud"});
  });
});
