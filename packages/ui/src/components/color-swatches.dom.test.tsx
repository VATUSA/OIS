// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

import {ColorSwatches} from "./color-swatches";

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

const SWATCHES = [
  {hex: "#f07178", label: "Red"},
  {hex: "#efc14d", label: "Amber"},
];

async function mount(value: string, onChange = vi.fn(), allowCustom = false) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  await act(async () => {
    root.render(
      <ColorSwatches swatches={SWATCHES} value={value} onChange={onChange} allowCustom={allowCustom} />,
    );
  });
  return host;
}

/** Sets the colour input through React's own value tracking. */
const pick = (input: HTMLInputElement, value: string) =>
  act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", {bubbles: true}));
  });

const swatches = (host: HTMLElement) => [...host.querySelectorAll<HTMLButtonElement>("button")];

describe("ColorSwatches (#698)", () => {
  it("names each swatch, not its hex", async () => {
    const host = await mount("#efc14d");
    expect(swatches(host).map((b) => [b.title, b.getAttribute("aria-label")])).toEqual([
      ["Red", "Red"],
      ["Amber", "Amber"],
    ]);
    expect(swatches(host)[1].getAttribute("aria-pressed")).toBe("true");
  });

  it("shows a saved colour outside the set as its own swatch, labelled with its hex", async () => {
    const host = await mount("#123abc");
    const extra = swatches(host)[2];
    expect(extra.title).toBe("#123abc");
    expect(extra.getAttribute("aria-pressed")).toBe("true");
  });

  it("takes a custom colour, but refuses one too dark to see on the map", async () => {
    const onChange = vi.fn();
    const host = await mount("#efc14d", onChange, true);
    const input = host.querySelector<HTMLInputElement>('input[type="color"]')!;

    await pick(input, "#08080a");
    expect(onChange).not.toHaveBeenCalled();
    expect(host.textContent).toContain("too dark");

    await pick(input, "#5EC8E5");
    expect(onChange).toHaveBeenCalledWith("#5ec8e5");
    expect(host.textContent).not.toContain("too dark");
  });
});
