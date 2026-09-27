// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

const platform = vi.hoisted(() => ({tauri: true, main: true}));
vi.mock("@/lib/platform", () => ({
  isTauri: () => platform.tauri,
  isMainWindow: async () => platform.main,
  can: () => true,
}));

// Stand-ins for the real notifiers: this is about *whether* they mount, not what they do.
vi.mock("@/components/notification-clicks", () => ({
  NotificationClicks: () => <span>clicks</span>,
}));
vi.mock("@/components/desktop-notifiers", () => ({
  DesktopNotifiers: () => <span>notifiers</span>,
}));

import {PrimaryWindowFeatures} from "./primary-window-features";

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
    root!.render(<PrimaryWindowFeatures />);
  });
  return host.innerHTML;
}

/**
 * `PrimaryWindowOnly` was tested on its own, but nothing tested that the app-global features are
 * actually inside it — taking the wrapper out of the root layout left the suite green. These cases
 * fail if any of these features is ever mounted unwrapped.
 */
describe("PrimaryWindowFeatures (VATUSA/OIS#350 review)", () => {
  it("mounts the OS-global features in the main window", async () => {
    const html = await rendered();
    expect(html).toContain("clicks");
    expect(html).toContain("notifiers");
  });

  it("mounts none of them in a route window", async () => {
    platform.main = false;
    expect(await rendered()).toBe("");
  });

  it("mounts them on the web build, which only ever has one window", async () => {
    platform.tauri = false;
    platform.main = false;
    const html = await rendered();
    expect(html).toContain("clicks");
    expect(html).toContain("notifiers");
  });
});
