// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

vi.mock("@/lib/api", () => ({ois: {GET: vi.fn()}}));

import {blankDraft, DraftEditor, draftFrom} from "./editors";
import {toUpsert, type Fca} from "@/lib/fca";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}

// The dark theme's swatch tokens, so the palette resolves to real, distinct colours under jsdom.
const TOKENS: Record<string, string> = {
  "series-1": "#5ec8e5",
  "series-2": "#43d089",
  "series-3": "#efc14d",
  "series-4": "#c792ea",
  "series-5": "#f07178",
  "series-6": "#7b9dff",
  "series-7": "#f5a83d",
  "series-8": "#a3d977",
  "ink-3": "#6b6b74",
};

beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  for (const [token, hex] of Object.entries(TOKENS)) {
    document.documentElement.style.setProperty(`--${token}`, hex);
  }
});

const roots: {root: ReturnType<typeof createRoot>; host: HTMLElement}[] = [];
afterEach(() => {
  for (const {root, host} of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
});

describe("the FCA colour picker (#698)", () => {
  it("offers the nine named swatches and a custom colour", async () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    roots.push({root, host});
    const qc = new QueryClient({defaultOptions: {queries: {retry: false}}});
    await act(async () => {
      root.render(
        <QueryClientProvider client={qc}>
          <DraftEditor
            draft={{...blankDraft(0), points: [[38, -77], [39, -77]]}}
            onChange={() => {}}
            onSave={() => {}}
            onRedraw={() => {}}
            onCancel={() => {}}
            saving={false}
          />
        </QueryClientProvider>,
      );
    });
    const named = [...host.querySelectorAll<HTMLButtonElement>("button[aria-pressed]")].map((b) => b.title);
    expect(named).toEqual(["Red", "Orange", "Amber", "Lime", "Green", "Cyan", "Blue", "Purple", "Gray"]);
    expect(host.querySelector('input[type="color"]')).not.toBeNull();
  });

  it("starts a new FCA on the first named swatch and cycles through all nine", () => {
    const colors = Array.from({length: 10}, (_, i) => blankDraft(i).color);
    expect(colors[0]).toBe("#f07178");
    expect(new Set(colors.slice(0, 9)).size).toBe(9);
    expect(colors[9]).toBe(colors[0]);
  });
});

// VATUSA/OIS#817: the freeze horizon is a per-FCA setting the editor sets and every save keeps.
describe("the freeze horizon field", () => {
  const stored = {
    id: "f-1", name: "FCA1", color: "#efc14d", artcc: "ZDC", points: [[38, -77], [39, -77]],
    dests: [], origins: [], fixes: [], scope: [], min_fl: null, max_fl: null, dir: "any",
    mode: "rate", rate: 30, mit: 15, freeze_min: 12, enabled: true,
  } as unknown as Fca;

  it("starts a new FCA at 30 min and loads an FCA's own value", () => {
    expect(blankDraft(0).freezeMin).toBe(30);
    expect(draftFrom(stored).freezeMin).toBe(12);
  });

  it("keeps the stored horizon when a toggle rebuilds the payload", () => {
    expect(toUpsert(stored).freeze_min).toBe(12);
  });

  it("edits the horizon through its input", async () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    roots.push({root, host});
    const onChange = vi.fn();
    const qc = new QueryClient({defaultOptions: {queries: {retry: false}}});
    await act(async () => {
      root.render(
        <QueryClientProvider client={qc}>
          <DraftEditor
            draft={draftFrom(stored)}
            onChange={onChange}
            onSave={() => {}}
            onRedraw={() => {}}
            onCancel={() => {}}
            saving={false}
          />
        </QueryClientProvider>,
      );
    });
    const label = [...host.querySelectorAll("label")].find((l) => l.textContent?.includes("Freeze horizon"));
    const input = label?.querySelector<HTMLInputElement>("input");
    expect(input?.value).toBe("12");
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    act(() => {
      setter.call(input, "45");
      input!.dispatchEvent(new Event("input", {bubbles: true}));
    });
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({freezeMin: 45}));
  });
});
