// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, beforeEach, describe, expect, it, vi} from "vitest";

/**
 * The sidebar's chrome row is where the window's own buttons live (#419), and on an undecorated
 * window they are the only way to close it. `window-controls.dom.test.tsx` proves the buttons
 * themselves behave; nothing proved they were still *mounted*, so deleting both `<WindowChromeSlot />`
 * from `ChromeRow` left the whole suite green while shipping a Windows window with no close button
 * (#419 review). This is that guard, for both sidebar states.
 *
 * It asserts the slot, not the three dots: on macOS the slot is deliberately empty — the OS paints the
 * real lights over it — so "the room is reserved" is the invariant that holds on every platform.
 */
const platform = vi.hoisted(() => ({windowControls: true, label: "main", macos: false}));
vi.mock("@/lib/platform", () => ({
  can: () => platform.windowControls,
  isMacOS: () => platform.macos,
  isMainWindow: async () => platform.label === "main",
  platform: () => (platform.windowControls ? "desktop" : "web"),
  availability: () => "available",
  capabilities: () => [],
  invokeDesktop: async () => undefined,
  windowLabel: async () => platform.label,
  isTauri: () => platform.windowControls,
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    label: platform.label,
    minimize: () => Promise.resolve(),
    toggleMaximize: () => Promise.resolve(),
    close: () => Promise.resolve(),
    isFocused: () => Promise.resolve(true),
    onFocusChanged: () => Promise.resolve(() => undefined),
  }),
}));

import {WindowChromeSlot} from "./window-controls";

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
  platform.windowControls = true;
  platform.label = "main";
  platform.macos = false;
});

async function render(node: React.ReactNode): Promise<HTMLElement> {
  const host = document.createElement("div");
  root = createRoot(host);
  await act(async () => {
    root!.render(node);
  });
  return host;
}

const slot = (host: HTMLElement) => host.querySelector<HTMLElement>("[data-window-chrome-slot]");

describe("the sidebar's window-chrome slot", () => {
  it("reserves room and draws the replica in an undecorated window", async () => {
    const host = await render(<WindowChromeSlot />);

    expect(slot(host)).not.toBeNull();
    // The buttons themselves, not just the space: this is the only way to close the window.
    expect(host.querySelectorAll("button")).toHaveLength(3);
    expect(host.querySelector(".traffic-lights")).not.toBeNull();
  });

  it("still reserves the room on macOS, where the OS paints the lights into it", async () => {
    // Empty on purpose — but the space must be held, or the back/forward pair slides under the real
    // traffic lights, which sit at fixed window coordinates the page cannot move.
    platform.macos = true;
    const host = await render(<WindowChromeSlot />);

    expect(slot(host)).not.toBeNull();
    expect(host.querySelectorAll("button")).toHaveLength(0);
  });

  /**
   * #419 review: the reservation has to be wider on macOS. The OS's lights measure 59px across —
   * three ~13px dots at a 23px pitch — against the replica's 52px (three 12px dots, two 8px gaps).
   * Reserving 52px on macOS put the green light over the Back button.
   */
  it("reserves more for the OS's lights than for the replica, because Apple's are wider", async () => {
    const replica = slot(await render(<WindowChromeSlot />))!.className;
    await act(() => root?.unmount());
    root = undefined;

    platform.macos = true;
    const native = slot(await render(<WindowChromeSlot />))!.className;

    expect(replica).toContain("w-[52px]");
    expect(native).toContain("w-[59px]");
  });

  it("reserves nothing in a route window, which has a native title bar", async () => {
    platform.label = "window--ops-idst";
    const host = await render(<WindowChromeSlot />);

    expect(slot(host)).toBeNull();
  });

  it("reserves nothing on the web build, where the browser owns the chrome", async () => {
    platform.windowControls = false;
    const host = await render(<WindowChromeSlot />);

    expect(slot(host)).toBeNull();
  });

  /**
   * The macOS lights are at fixed *window* coordinates the page cannot move, so the chrome row has to
   * start at a fixed window y. Anything in flow above the shell breaks that: `<UpdateBanner />` above
   * `<Frame>` pushed the row down ~37px whenever an update was staged, putting the real lights on the
   * banner and leaving an empty reserved gap in the row (#419 review).
   */
  it("keeps the update banner inside the frame, so nothing in flow shifts the chrome row", async () => {
    const {readFile} = await import("node:fs/promises");
    const {resolve} = await import("node:path");
    const source = await readFile(
      resolve(process.cwd(), "src/components/shell/app-shell.tsx"),
      "utf8",
    );
    const frame = source.indexOf("<Frame");
    const banner = source.indexOf("<UpdateBanner />");

    expect(frame).toBeGreaterThan(-1);
    expect(banner).toBeGreaterThan(frame);
  });

  it("keeps the slot ahead of the navigation, where the window's top-left is", async () => {
    // Source-level, because jsdom has no layout: the order in the row is what puts the buttons at the
    // window's leading edge, and back/forward right of them.
    // Read off disk rather than through `import.meta.url`, which Vitest serves over http.
    const {readFile} = await import("node:fs/promises");
    const {resolve} = await import("node:path");
    const source = await readFile(
      resolve(process.cwd(), "src/components/shell/app-sidebar.tsx"),
      "utf8",
    );
    const collapsed = source.indexOf("Expand sidebar");
    const expanded = source.indexOf('label="Back"');
    const slots = [...source.matchAll(/<WindowChromeSlot\b/g)].map((m) => m.index);

    // One per ChromeRow branch — collapsed and expanded — and each ahead of that branch's controls.
    expect(slots).toHaveLength(2);
    expect(slots[0]).toBeLessThan(collapsed);
    expect(slots[1]).toBeGreaterThan(collapsed);
    expect(slots[1]).toBeLessThan(expanded);
  });
});
