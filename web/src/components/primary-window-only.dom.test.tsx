// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {beforeAll, describe, expect, it, vi} from "vitest";

const env = vi.hoisted(() => ({desktop: true, main: true}));
vi.mock("@/lib/platform", () => ({
  isTauri: () => env.desktop,
  isMainWindow: async () => env.main,
}));

import {PrimaryWindowOnly} from "./primary-window-only";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

async function rendered(): Promise<string> {
  const host = document.createElement("div");
  const root = createRoot(host);
  await act(async () => root.render(<PrimaryWindowOnly><span>tray</span></PrimaryWindowOnly>));
  await act(async () => new Promise((r) => setTimeout(r, 0)));
  const html = host.innerHTML;
  act(() => root.unmount());
  return html;
}

// What keeps the tray, the notifier and click handling to ONE window: route windows run the full
// shell, and each copy raced to rebuild the one tray menu (VATUSA/OIS#351 review).
describe("PrimaryWindowOnly", () => {
  it("renders in the main window", async () => {
    Object.assign(env, {desktop: true, main: true});
    expect(await rendered()).toContain("tray");
  });

  it("renders nothing in a route window or pop-out", async () => {
    Object.assign(env, {desktop: true, main: false});
    expect(await rendered()).toBe("");
  });

  it("renders on the web build, which only ever has one window", async () => {
    Object.assign(env, {desktop: false, main: false});
    expect(await rendered()).toContain("tray");
  });
});
