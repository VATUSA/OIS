// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

const platform = vi.hoisted(() => ({tauri: true, main: true}));
vi.mock("@/lib/platform", () => ({
  isTauri: () => platform.tauri,
  isMainWindow: async () => platform.main,
}));

import {PrimaryWindowOnly} from "./primary-window-only";

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
  platform.tauri = true;
  platform.main = true;
});

async function rendered(): Promise<string> {
  const host = document.createElement("div");
  root = createRoot(host);
  await act(async () => {
    root!.render(
      <PrimaryWindowOnly>
        <span>notifier</span>
      </PrimaryWindowOnly>,
    );
  });
  return host.innerHTML;
}

describe("PrimaryWindowOnly (VATUSA/OIS#350 review)", () => {
  it("renders in the desktop app's main window", async () => {
    expect(await rendered()).toContain("notifier");
  });

  // Mounted in every window, one ground stop raised one OS notification per open window, and one
  // click made every window raise and navigate itself.
  it("renders nothing in any other window", async () => {
    platform.main = false;
    expect(await rendered()).toBe("");
  });

  it("renders on the web build, which only ever has the one window", async () => {
    platform.tauri = false;
    platform.main = false;
    expect(await rendered()).toContain("notifier");
  });
});
