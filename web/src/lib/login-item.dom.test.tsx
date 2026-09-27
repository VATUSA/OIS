// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, beforeEach, describe, expect, it, vi} from "vitest";

const os = vi.hoisted(() => ({enabled: true, calls: [] as string[]}));
vi.mock("@tauri-apps/plugin-autostart", () => ({
  isEnabled: async () => {
    os.calls.push("isEnabled");
    return os.enabled;
  },
  enable: async () => {
    os.calls.push("enable");
    os.enabled = true;
  },
  disable: async () => {
    os.calls.push("disable");
    os.enabled = false;
  },
}));

import {useLoginItem} from "./tray";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

let hook: ReturnType<typeof useLoginItem>;
function Probe() {
  hook = useLoginItem();
  return null;
}
let root: ReturnType<typeof createRoot> | undefined;
const settle = () => act(async () => new Promise((r) => setTimeout(r, 0)));

beforeEach(() => {
  os.calls = [];
});
afterEach(() => {
  act(() => root?.unmount());
  root = undefined;
});

async function mount() {
  root = createRoot(document.createElement("div"));
  await act(async () => root!.render(<Probe />));
  await settle();
}

describe("useLoginItem (VATUSA/OIS#351 review)", () => {
  // The OS is the source of truth. Reconciling an account setting at launch disabled the login item
  // while settings loaded, followed the account to other machines, and undid a removal in the OS.
  it("only reads the OS on mount — it never writes the login item by itself", async () => {
    os.enabled = true;
    await mount();
    expect(hook.enabled).toBe(true);
    expect(os.calls).toEqual(["isEnabled"]);
  });

  it("reports a login item the user removed in the OS as off, and leaves it off", async () => {
    os.enabled = false;
    await mount();
    expect(hook.enabled).toBe(false);
    expect(os.calls).not.toContain("enable");
  });

  it("switches the login item on and off, showing what the OS then reports", async () => {
    os.enabled = false;
    await mount();

    await act(async () => hook.setEnabled(true));
    await settle();
    expect(os.calls).toContain("enable");
    expect(hook.enabled).toBe(true);

    await act(async () => hook.setEnabled(false));
    await settle();
    expect(os.calls).toContain("disable");
    expect(hook.enabled).toBe(false);
  });
});
