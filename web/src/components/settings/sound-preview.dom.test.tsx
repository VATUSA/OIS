// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, beforeEach, describe, expect, it, vi} from "vitest";

/**
 * The Sounds row's preview button (#404).
 *
 * Its whole reason to exist is telling the user *which* file they heard, so the reported answer is the
 * behaviour under test — not merely that something played.
 */
const settings = vi.hoisted(() => ({} as Record<string, unknown>));
vi.mock("@/lib/settings", () => ({
  useSetting: <T,>(key: string, fallback: T) => ({
    value: (settings[key] as T | undefined) ?? fallback,
  }),
}));
const previewAlertSound = vi.hoisted(() =>
  vi.fn((_category: string, _volume?: string) => Promise.resolve("bundled" as string)),
);
vi.mock("@/lib/sounds", () => ({previewAlertSound}));

import {SoundPreviewButton} from "./sound-preview";

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
  previewAlertSound.mockClear().mockResolvedValue("bundled");
  for (const key of Object.keys(settings)) delete settings[key];
});

async function press(): Promise<HTMLElement> {
  const host = document.createElement("div");
  root = createRoot(host);
  await act(async () => {
    root!.render(<SoundPreviewButton category="restrictions" />);
  });
  await act(async () => host.querySelector("button")!.click());
  return host;
}

describe("SoundPreviewButton", () => {
  it("says the replacement was used when it was", async () => {
    previewAlertSound.mockResolvedValue("replacement");
    expect((await press()).textContent).toContain("Your file");
  });

  it("says the bundled default was used, which is the silent fallback made visible", async () => {
    previewAlertSound.mockResolvedValue("bundled");
    const host = await press();

    expect(host.textContent).toContain("Bundled default");
    expect(host.textContent).not.toContain("Your file");
  });

  it("says so when nothing could be played", async () => {
    previewAlertSound.mockResolvedValue("none");
    expect((await press()).textContent).toContain("Couldn't play it");
  });

  it("says nothing before it is pressed", async () => {
    const host = document.createElement("div");
    root = createRoot(host);
    await act(async () => {
      root!.render(<SoundPreviewButton category="restrictions" />);
    });

    expect(host.textContent).toBe("");
    expect(previewAlertSound).not.toHaveBeenCalled();
  });

  it("plays at the volume that row is set to, not a default", async () => {
    settings["sounds.restrictions.volume"] = "loud";
    await press();

    expect(previewAlertSound).toHaveBeenCalledWith("restrictions", "loud");
  });

  it("reports through a live region, so it is announced and not just drawn", async () => {
    const host = await press();
    expect(host.querySelector("[aria-live]")).not.toBeNull();
  });

  // A repeat press with the same answer changed no text, so the live region announced nothing and a
  // sighted user couldn't tell the second press ran at all (VATUSA/OIS#404 review).
  it("clears the last answer while a new press is playing", async () => {
    const host = await press();
    expect(host.textContent).toContain("Bundled default");

    let finish: (source: string) => void = () => undefined;
    previewAlertSound.mockReturnValueOnce(new Promise((resolve) => (finish = resolve)));
    await act(async () => host.querySelector("button")!.click());
    expect(host.textContent).not.toContain("Bundled default");

    await act(async () => finish("bundled"));
    expect(host.textContent).toContain("Bundled default");
  });
});
