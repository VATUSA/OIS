// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

// The build's release stage (#841). The real one comes from the repo-root STAGE file through the
// Vite define; mocked so the default-prop test does not change meaning when STAGE does.
const build = vi.hoisted(() => ({stage: "beta" as "alpha" | "beta" | "rc" | "stable"}));
vi.mock("@/lib/build-stage", () => ({
  get BUILD_STAGE() {
    return build.stage;
  },
}));

import {ReleaseStageChip} from "./release-stage-chip";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

let root: ReturnType<typeof createRoot> | undefined;
let host: HTMLDivElement | undefined;
afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
  root = undefined;
  host = undefined;
  document.body.innerHTML = "";
  build.stage = "beta";
});

function mount(node: React.ReactNode): HTMLDivElement {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => root!.render(node));
  return host;
}

/** The hover-titled wrapper and the pill inside it. */
function parts(el: HTMLElement) {
  const wrapper = el.querySelector<HTMLElement>("[data-release-stage]")!;
  return {wrapper, pill: wrapper.firstElementChild as HTMLElement};
}

describe("ReleaseStageChip (#841)", () => {
  it('renders an uppercase "Alpha" with the alpha hover text and the warn tone for alpha', () => {
    const {wrapper, pill} = parts(mount(<ReleaseStageChip stage="alpha" />));
    expect(pill.textContent).toBe("Alpha");
    expect(pill.className).toContain("uppercase");
    expect(wrapper.getAttribute("title")).toBe("Alpha: expect rough edges and frequent changes");
    expect(pill.className).toContain("bg-warning-soft");
    expect(pill.className).toContain("text-warning");
  });

  it("renders Beta with the beta hover text and the brand tone for beta", () => {
    const {wrapper, pill} = parts(mount(<ReleaseStageChip stage="beta" />));
    expect(pill.textContent).toBe("Beta");
    expect(wrapper.getAttribute("title")).toBe("Beta: early access, still stabilizing");
    expect(pill.className).toContain("bg-brand-soft");
  });

  it("renders RC with the release-candidate hover text and the neutral tone for rc", () => {
    const {wrapper, pill} = parts(mount(<ReleaseStageChip stage="rc" />));
    expect(pill.textContent).toBe("RC");
    expect(wrapper.getAttribute("title")).toBe("Release candidate");
    expect(pill.className).toContain("bg-chip");
  });

  it("renders nothing at all for stable", () => {
    // Positive control in the same mount shape: alpha renders.
    expect(mount(<ReleaseStageChip stage="alpha" />).innerHTML).not.toBe("");
    act(() => root!.unmount());
    expect(mount(<ReleaseStageChip stage="stable" />).innerHTML).toBe("");
  });

  it("uses the build's stage when no stage prop is passed", () => {
    build.stage = "beta";
    expect(parts(mount(<ReleaseStageChip />)).pill.textContent).toBe("Beta");
    act(() => root!.unmount());
    build.stage = "stable";
    expect(mount(<ReleaseStageChip />).innerHTML).toBe("");
  });

  it("keeps the label and hover text in the compact variant, at the smaller size", () => {
    const {wrapper, pill} = parts(mount(<ReleaseStageChip stage="alpha" compact />));
    expect(pill.textContent).toBe("Alpha");
    expect(wrapper.getAttribute("title")).toBe("Alpha: expect rough edges and frequent changes");
    expect(pill.className).toContain("text-[10px]");
    expect(pill.className).not.toContain("text-xs");
  });
});
