// @vitest-environment jsdom
//
// VATUSA/OIS#816 AC4: IDST names the Set input "Ready time" and its result "Release time
// (wheels-up)", and after a Set reports how far past the ready time the release landed.
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, beforeEach, describe, expect, it, vi} from "vitest";

const api = vi.hoisted(() => ({ POST: vi.fn(), DELETE: vi.fn() }));
vi.mock("@/lib/api", () => ({ ois: api, API_BASE: "", DOCS_URL: "" }));

import type {IdstFlight} from "@/lib/idst";
import {SelectedPanel} from "./index";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});
beforeEach(() => {
  api.POST.mockReset();
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date("2026-10-10T13:00:00Z"));
});

const roots: { root: ReturnType<typeof createRoot>; host: HTMLElement }[] = [];
afterEach(() => {
  vi.useRealTimers();
  for (const { root, host } of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
  document.body.innerHTML = "";
});

const flight = (over: Partial<IdstFlight>): IdstFlight => ({
  callsign: "AAL1",
  dep: "KJFK",
  arr: "KDCA",
  aircraft_type: "B738",
  status: "ground",
  fca_id: "f1",
  fca_name: "ZDC WEST",
  seq: 1,
  delay_min: 0,
  cross_time: "2026-10-10T14:30:00Z",
  edct: "2026-10-10T14:00:00Z",
  released: false,
  released_by_machine: null,
  runway: null,
  runway_source: null,
  ...over,
});

function mount(selected: IdstFlight) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity }, mutations: { retry: false } },
  });
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({ root, host });
  const render = (f: IdstFlight) =>
    act(() =>
      root.render(
        <QueryClientProvider client={qc}>
          <ToastProvider>
            <SelectedPanel selected={f} released={[]} canEdit />
          </ToastProvider>
        </QueryClientProvider>,
      ),
    );
  render(selected);
  return { host, render };
}

const rowLabels = (host: HTMLElement) => [...host.querySelectorAll("dt")].map((d) => d.textContent);

describe("IDST SelectedPanel (VATUSA/OIS#816)", () => {
  it("labels the Set input \"Ready time\", tied to the input", () => {
    const { host } = mount(flight({}));
    const label = [...host.querySelectorAll("label")].find((l) => l.textContent === "Ready time");
    expect(label).toBeDefined();
    const input = host.querySelector(`#${label!.htmlFor}`);
    expect(input?.getAttribute("placeholder")).toBe("HHMMz");
  });

  it("names an unreleased flight's time \"Proposed release\" and a released one's \"Release time (wheels-up)\"", () => {
    const { host, render } = mount(flight({}));
    expect(rowLabels(host)).toContain("Proposed release");
    render(flight({ released: true }));
    expect(rowLabels(host)).toContain("Release time (wheels-up)");
    expect(host.textContent).not.toContain("EDCT");
  });

  it("after a Set shows the delay of the assigned release against the typed ready time", async () => {
    // Ready 1400Z; the server gave a release at 14:07Z (its 14:30 slot was taken).
    api.POST.mockResolvedValue({
      data: [{ callsign: "AAL1", released: true, edct: "2026-10-10T14:07:00Z", cross_time: "2026-10-10T14:37:00Z" }],
      error: undefined,
      response: { status: 200, ok: true },
    });
    const { host, render } = mount(flight({}));
    const input = host.querySelector<HTMLInputElement>("#idst-ready")!;
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    act(() => {
      setter.call(input, "1400");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    const set = [...host.querySelectorAll("button")].find((b) => b.textContent === "Set")!;
    await act(async () => {
      set.click();
      await new Promise((r) => setTimeout(r, 0));
    });
    expect(api.POST).toHaveBeenCalledWith(
      "/api/v1/flow/fcas/{id}/release/{callsign}",
      expect.objectContaining({ body: { ready: "1400" } }),
    );
    // The IDST refetch then shows the frozen release in the panel's own rows.
    render(flight({ released: true, edct: "2026-10-10T14:07:00Z", cross_time: "2026-10-10T14:37:00Z" }));
    const row = host.querySelector("[data-set-result]");
    expect(row?.textContent).toBe("Delay after ready 1400z+7m");
    expect(host.textContent).toContain("1407z");
    expect(host.textContent).toContain("1437z");
  });
});
