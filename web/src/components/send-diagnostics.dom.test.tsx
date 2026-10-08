// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({invoke: (...a: unknown[]) => invoke(...a)}));
vi.mock("@tauri-apps/api/window", () => ({getCurrentWindow: () => ({label: "popout-fca-1"})}));

import {SendDiagnosticsButton} from "./send-diagnostics";
import {ErrorBoundary} from "./error-boundary";
import {log, logTail, resetLogTail} from "@/lib/logger";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}

beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

const roots: {root: ReturnType<typeof createRoot>; host: HTMLElement}[] = [];
afterEach(() => {
  for (const {root, host} of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
  document.body.innerHTML = "";
  delete window.__TAURI_INTERNALS__;
  invoke.mockReset();
  resetLogTail();
  vi.restoreAllMocks();
});

async function mount(node: React.ReactNode) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  await act(async () => {
    root.render(<ToastProvider>{node}</ToastProvider>);
  });
  return host;
}

const button = (label: string) =>
  [...document.querySelectorAll("button")].find((b) => b.textContent === label);

describe("SendDiagnosticsButton", () => {
  it("is not offered on the web build", async () => {
    const host = await mount(<SendDiagnosticsButton />);
    expect(host.textContent).toBe("");
  });

  /** #629 AC3: the webview's half of the report — note, window, route, its log tail — reaches the
   * desktop command, which adds the rest and sends it. */
  it("sends the note and the webview's context through the desktop command", async () => {
    window.__TAURI_INTERNALS__ = {};
    invoke.mockResolvedValue("report-1");
    log("warn", "realtime dropped", "console");
    await mount(<SendDiagnosticsButton />);

    await act(async () => button("Send diagnostics…")!.click());
    const note = document.querySelector("textarea")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(note, "map went white");
      note.dispatchEvent(new Event("input", {bubbles: true}));
    });
    await act(async () => button("Send")!.click());

    const call = invoke.mock.calls.find(([command]) => command === "send_diagnostics");
    expect(call).toBeDefined();
    const args = call![1] as {apiBase: string; note: string; context: Record<string, unknown>};
    expect(args.note).toBe("map went white");
    expect(args.apiBase).toBeTruthy();
    expect(args.context.window_label).toBe("popout-fca-1");
    expect(args.context.route).toBe(window.location.pathname);
    expect(args.context.capabilities).toMatchObject({diagnostics: true});
    expect(args.context).toHaveProperty("webgl2");
    expect(args.context).toHaveProperty("realtime");
    expect((args.context.log_tail as string[]).join("\n")).toMatch(/realtime dropped/);
  });
});

describe("ErrorBoundary", () => {
  /** #629 AC2: a render crash leaves a log entry, not only a white screen. */
  it("logs a render crash and shows a way back", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    const Boom = () => {
      throw new Error("kaboom in render");
    };

    const host = await mount(
      <ErrorBoundary>
        <Boom />
      </ErrorBoundary>,
    );

    expect(host.textContent).toContain("Something went wrong");
    expect(logTail().join("\n")).toMatch(/\[error\]\[react\] render crash: Error: kaboom in render/);
  });
});
