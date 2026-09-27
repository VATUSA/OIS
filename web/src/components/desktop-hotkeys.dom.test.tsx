// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot, type Root} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";

/**
 * One registry shared by every mounted component — which is what the OS actually gives the app.
 * `unregisterAll` is process-wide, not per-window, and that is the whole point of these tests.
 */
const registry = new Set<string>();
let windowLabel = "main";
const warning = vi.fn();

vi.mock("@tauri-apps/plugin-global-shortcut", () => ({
  register: async (accelerator: string) => {
    registry.add(accelerator);
  },
  unregisterAll: async () => {
    registry.clear();
  },
}));
/** Tauri's app-wide event bus: an event emitted in any window reaches listeners in every window. */
const bus = new Map<string, Set<() => void>>();
vi.mock("@tauri-apps/api/event", () => ({
  emit: async (event: string) => {
    for (const handler of bus.get(event) ?? []) handler();
  },
  listen: async (event: string, handler: () => void) => {
    if (!bus.has(event)) bus.set(event, new Set());
    bus.get(event)!.add(handler);
    return () => bus.get(event)!.delete(handler);
  },
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({label: windowLabel}),
}));
vi.mock("@ois/ui", () => ({useToast: () => ({warning})}));
vi.mock("@/lib/popout", () => ({openRouteWindow: async () => true}));
vi.mock("@/lib/tray", () => ({showMainWindow: async () => {}}));
vi.mock("@/lib/alerts", () => ({dismissAllAlerts: () => {}}));

let settings: {isSuccess: boolean; data: Record<string, unknown>} = {isSuccess: true, data: {}};
vi.mock("@/lib/settings", () => ({useSettings: () => settings}));

import {DesktopHotkeys} from "./desktop-hotkeys";
import {resumeHotkeys, suspendHotkeys} from "@/lib/hotkeys";

/** Mounts the component the way a Tauri window would, and lets the 900ms settle elapse. */
let queryClient = new QueryClient();

async function mountWindow(label: string): Promise<Root> {
  windowLabel = label;
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  await act(async () => {
    root.render(
      <QueryClientProvider client={queryClient}>
        <DesktopHotkeys />
      </QueryClientProvider>,
    );
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1500);
  });
  return root;
}

beforeEach(() => {
  vi.useFakeTimers();
  registry.clear();
  bus.clear();
  queryClient = new QueryClient();
  warning.mockReset();
  windowLabel = "main";
  window.__TAURI_INTERNALS__ = {};
  settings = {isSuccess: true, data: {"hotkeys.focus": "Command+Shift+O"}};
});

afterEach(() => {
  vi.useRealTimers();
  delete window.__TAURI_INTERNALS__;
});

describe("DesktopHotkeys", () => {
  it("registers the user's bindings in the main window", async () => {
    const main = await mountWindow("main");
    expect([...registry]).toEqual(["Command+Shift+O"]);
    await act(async () => main.unmount());
  });

  it("registers nothing from a route window", async () => {
    // RootLayout renders in every whole-route window (#350), and a shortcut belongs to the process,
    // not to a window — so only one window may own them.
    const route = await mountWindow("window-/ops/tmu");
    expect([...registry]).toEqual([]);
    await act(async () => route.unmount());
  });

  it("keeps the main window's shortcuts when a route window closes", async () => {
    // The regression this guard exists for: the jump shortcuts themselves open route windows, and
    // `restoreWindows()` reopens them on every launch. Without the guard, closing one ran
    // `unregisterAll()` and every global shortcut went dead until a settings edit or a restart.
    const main = await mountWindow("main");
    const route = await mountWindow("window-/ops/tmu");

    await act(async () => route.unmount());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1500);
    });

    expect([...registry]).toEqual(["Command+Shift+O"]);
    await act(async () => main.unmount());
  });

  it("releases the shortcuts when the main window itself goes away", async () => {
    const main = await mountWindow("main");
    await act(async () => main.unmount());
    expect([...registry]).toEqual([]);
  });

  it("names both reasons when shortcuts are refused for different ones", async () => {
    // A modifier-less binding used to hide an "already taken" one completely, and the once-per
    // -configuration dedupe then meant that conflict was never reported again.
    const shortcut = await import("@tauri-apps/plugin-global-shortcut");
    vi.spyOn(shortcut, "register").mockRejectedValueOnce(new Error("already registered"));
    settings = {
      isSuccess: true,
      data: {"hotkeys.focus": "O", "hotkeys.tmu": "Command+Space"},
    };

    const main = await mountWindow("main");

    expect(warning).toHaveBeenCalledTimes(1);
    const description = warning.mock.calls[0]![1].description as string;
    expect(description).toContain("O");
    expect(description).toContain("Command+Space");
    await act(async () => main.unmount());
  });
});

describe("recording a shortcut, from any window (VATUSA/OIS#352 review)", () => {
  const settle = () => act(async () => void (await vi.advanceTimersByTimeAsync(1500)));

  // Settings can be open in a route window. Its suspend released every shortcut app-wide, and its
  // resume only reached listeners in that window — so nothing ever took them back.
  it("takes the shortcuts back when a field in another window stops recording", async () => {
    const root = await mountWindow("main");
    expect(registry.has("Command+Shift+O")).toBe(true);

    await act(async () => void (await suspendHotkeys())); // the field lives in a route window
    await settle();
    expect(registry.size).toBe(0);

    await act(async () => void (await resumeHotkeys()));
    await settle();
    expect(registry.has("Command+Shift+O")).toBe(true);
    act(() => root.unmount());
  });

  // A pending re-registration used to put every shortcut back while a field was still recording,
  // so pressing an already-bound combination fired it instead of being recorded.
  it("keeps the shortcuts released for as long as a field is recording", async () => {
    const root = await mountWindow("main");

    await act(async () => void (await resumeHotkeys())); // previous field finished: apply pending
    await act(async () => void (await suspendHotkeys())); // next field starts recording at once
    await settle(); // the pending apply's timer comes due

    expect(registry.size).toBe(0);
    act(() => root.unmount());
  });

  // A field in a route window saves into *that* window's settings cache; the owner has to re-read,
  // or it re-registers the binding it had before the edit.
  it("re-reads the settings when a field finishes, so an edit made in another window lands", async () => {
    const root = await mountWindow("main");
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");

    await act(async () => void (await resumeHotkeys()));

    expect(invalidate).toHaveBeenCalledWith({queryKey: ["preferences", "settings"]});
    act(() => root.unmount());
  });
});
