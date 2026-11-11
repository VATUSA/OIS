// @vitest-environment jsdom
//
// VATUSA/OIS#814 AC6: a time that rests on a ground flight's proposed wheels-up is marked, by a letter as
// well as a color, and nothing else is.
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, describe, expect, it} from "vitest";

import {PROPOSED_TITLE, ProposedTime} from "./proposed-time";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

const roots: { root: ReturnType<typeof createRoot>; host: HTMLElement }[] = [];
afterEach(() => {
  for (const { root, host } of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
});

function mount(node: React.ReactNode) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({ root, host });
  act(() => root.render(node));
  return host.firstElementChild as HTMLElement;
}

const TIME = "2026-10-06T12:34:00Z";

describe("ProposedTime (#814)", () => {
  it("marks a proposed time with a P, a screen-reader word and the tooltip", () => {
    const el = mount(<ProposedTime iso={TIME} proposed className="text-ink-2" />);
    expect(el.className).toBe("text-ink-2");
    expect(el.title).toBe(PROPOSED_TITLE);
    expect(el.textContent).toBe("1234zP proposed");
    const letter = el.querySelector('[aria-hidden="true"]');
    expect(letter?.textContent).toBe("P");
    expect(letter?.className).toContain("text-flight-proposed");
    expect(el.querySelector(".sr-only")?.textContent).toBe(" proposed");
  });

  it("shows a held time unmarked", () => {
    const el = mount(<ProposedTime iso={TIME} proposed={false} />);
    expect(el.textContent).toBe("1234z");
    expect(el.title).toBe("");
    expect(el.querySelector('[aria-hidden="true"]')).toBeNull();
  });

  it.each([null, undefined, "not-a-time"])("shows no marker beside a dash for %s", (iso) => {
    const el = mount(<ProposedTime iso={iso} proposed />);
    expect(el.textContent).toBe("—");
    expect(el.querySelector(".sr-only")).toBeNull();
    expect(el.title).toBe("");
  });
});
