// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

import {SectorGrid, type SectorGridRow} from "./sector-grid";

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
});

/** 24 quarter-hours from 1400Z. */
const START = Date.UTC(2026, 9, 5, 14, 0);
const BINS = Array.from({length: 24}, (_, i) => START + i * 15 * 60_000);

function rows(n: number): SectorGridRow[] {
  return Array.from({length: n}, (_, r) => ({
    id: `ZLA${String(r + 10).padStart(2, "0")}`,
    limit: 10,
    cells: BINS.map((_, i) => ({combined: (r + i) % 14, active: (r + i) % 9, level: "ok" as const})),
  }));
}

async function mount(data: SectorGridRow[], onLimitChange?: (id: string, limit: number) => void) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  await act(async () => {
    root.render(<SectorGrid rows={data} binStarts={BINS} caption="ZLA enroute" onLimitChange={onLimitChange} />);
  });
  return host;
}

const cells = (host: HTMLElement) => [...host.querySelectorAll<HTMLElement>("[data-level]")];

/** Sets an input through React's own value tracking. */
const type = (input: HTMLInputElement, value: string) =>
  act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", {bubbles: true}));
  });
const key = (input: HTMLInputElement, k: string) =>
  act(async () => {
    input.dispatchEvent(new KeyboardEvent("keydown", {key: k, bubbles: true}));
  });

describe("SectorGrid (#724)", () => {
  it("draws 30 sectors across 24 bins inside its own scroller, leading columns sticky", async () => {
    const host = await mount(rows(30));
    expect(host.querySelectorAll("tbody tr")).toHaveLength(30);
    expect(cells(host)).toHaveLength(30 * 24);
    // The grid scrolls, not the page: the table sits in an overflow-x container.
    expect(host.firstElementChild!.className).toContain("overflow-x-auto");
    const firstRow = host.querySelector("tbody tr")!;
    const [sector, limit] = [firstRow.querySelector("th")!, firstRow.querySelector("td")!];
    expect(sector.className).toMatch(/\bsticky\b/);
    expect(limit.className).toMatch(/\bsticky\b/);
    expect(sector.className).toContain("left-0");
    expect(limit.className).toContain("left-28");
  });

  it("hovers each cell with both figures: combined peak and airborne alone", async () => {
    const host = await mount([
      {id: "ZLA25", limit: 10, cells: BINS.map((_, i) => ({combined: i === 1 ? 12 : 3, active: i === 1 ? 9 : 1, level: "watch" as const}))},
    ]);
    const cell = cells(host)[1];
    expect(cell.title).toBe("ZLA25 1415Z · peak 12 (airborne 9) vs limit 10");
    expect(cell.getAttribute("aria-label")).toBe(cell.title);
    expect(cell.textContent).toBe("12");
  });

  it("labels the time axis with each bin's start in HHMM Zulu", async () => {
    const host = await mount(rows(1));
    const footer = [...host.querySelectorAll("tfoot td")].map((c) => c.textContent).filter(Boolean);
    expect(footer.slice(0, 4)).toEqual(["1400", "1415", "1430", "1445"]);
    expect(footer).toHaveLength(24);
  });

  it("draws each cell's level, as given", async () => {
    const levels = ["ok", "watch", "over"] as const;
    const host = await mount([
      {id: "ZLA25", limit: 10, cells: BINS.map((_, i) => ({combined: 1, active: 1, level: levels[i % 3]}))},
    ]);
    expect(cells(host).slice(0, 3).map((c) => c.dataset.level)).toEqual(["ok", "watch", "over"]);
    expect(cells(host)[2].className).toContain("level-over");
  });

  it("labels a combined row with the sectors it carries, and leaves a plain row unlabelled", async () => {
    const [plain] = rows(1);
    const host = await mount([{...plain, id: "ZLA20", carries: ["ZLA21", "ZLA22"]}, plain]);
    const [combined, single] = [...host.querySelectorAll("tbody th")];
    const label = combined.querySelector<HTMLElement>("[data-carries]")!;
    expect(label.textContent).toBe("carries +ZLA21 +ZLA22");
    expect(label.title).toBe("ZLA20 carries ZLA21, ZLA22");
    // Under the id, inside the sticky column's width, so the limit column's offset still holds.
    expect(label.className).toMatch(/\bblock\b/);
    expect(label.className).toMatch(/\btruncate\b/);
    expect(single.querySelector("[data-carries]")).toBeNull();
    expect(single.textContent).toBe("ZLA10");
  });

  it("offers no affordance on the limit when the viewer can't edit it", async () => {
    const host = await mount(rows(1));
    const limit = host.querySelector("tbody td")!;
    expect(limit.querySelector("input, button, [tabindex], [role=button]")).toBeNull();
    expect(limit.innerHTML).not.toMatch(/hover:|cursor-|focus:/);
    expect(limit.textContent).toBe("10");
  });

  it("commits an edited limit on Enter, refuses a non-positive one, and reverts on Escape", async () => {
    const onLimitChange = vi.fn();
    const host = await mount(rows(1), onLimitChange);
    const input = host.querySelector<HTMLInputElement>("tbody td input")!;
    expect(input.getAttribute("aria-label")).toBe("Limit for ZLA10");

    await act(async () => input.focus());
    await type(input, "14");
    await key(input, "Escape");
    expect(onLimitChange).not.toHaveBeenCalled();
    expect(input.value).toBe("10");

    await act(async () => input.focus());
    await type(input, "0");
    await act(async () => input.blur());
    expect(onLimitChange).not.toHaveBeenCalled();
    expect(input.value).toBe("10");

    await act(async () => input.focus());
    await type(input, "14");
    await key(input, "Enter");
    expect(onLimitChange).toHaveBeenCalledWith("ZLA10", 14);
  });

  it("puts back only the input whose reset count moved; a draft in another row survives", async () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    roots.push({root, host});
    const render = (limitResets: Record<string, number>) =>
      act(async () => {
        root.render(
          <SectorGrid rows={rows(2)} binStarts={BINS} caption="ZLA enroute" onLimitChange={() => {}} limitResets={limitResets} />,
        );
      });
    const input = (id: string) => host.querySelector<HTMLInputElement>(`input[aria-label="Limit for ${id}"]`)!;
    await render({});
    await type(input("ZLA10"), "20");
    await type(input("ZLA11"), "17");
    await render({ZLA10: 1});
    expect(input("ZLA10").value).toBe("10");
    expect(input("ZLA11").value).toBe("17");
  });
});
