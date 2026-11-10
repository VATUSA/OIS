// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {
  RouterProvider,
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
} from "@tanstack/react-router";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

// The release-stage chip on the signed-out landing hero and beside the footer version (#841).
// The chip's label, tones and hover text are pinned in release-stage-chip.dom.test.tsx.

const build = vi.hoisted(() => ({stage: "alpha" as "alpha" | "beta" | "rc" | "stable"}));
vi.mock("@/lib/build-stage", () => ({
  get BUILD_STAGE() {
    return build.stage;
  },
}));
vi.mock("@/lib/auth", () => ({
  useMe: () => ({data: null}),
  useLogin: () => ({mutate: () => undefined}),
  useSignInPending: () => false,
}));

import {LandingPage} from "./landing";

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
  document.body.innerHTML = "";
  build.stage = "alpha";
});

async function renderLanding() {
  const rootRoute = createRootRoute({
    component: () => (
      <ToastProvider>
        <LandingPage />
      </ToastProvider>
    ),
  });
  const splat = createRoute({getParentRoute: () => rootRoute, path: "$"});
  const router = createRouter({
    routeTree: rootRoute.addChildren([splat]),
    history: createMemoryHistory({initialEntries: ["/"]}),
  });
  const host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  await act(async () => {
    await router.load();
    root!.render(<RouterProvider router={router} />);
  });
}

const heroTitle = () => document.querySelector<HTMLElement>("[data-hero-title]")!;
/** The footer's version link: the anchor whose text starts with "v" and a digit. */
const versionLink = () =>
  [...document.querySelectorAll<HTMLAnchorElement>("footer a")].find((a) => /^v\d/.test(a.textContent ?? ""))!;

describe("LandingPage release stage (#841)", () => {
  it("renders the chip beside the hero h1, inside the hero and not only in the footer", async () => {
    await renderLanding();
    const title = heroTitle();
    expect(title.querySelector("h1")!.textContent!.trim()).toBe("OIS");
    const chip = title.querySelector("[data-release-stage]")!;
    expect(chip).not.toBeNull();
    expect(chip.textContent).toBe("Alpha");
    expect(chip.previousElementSibling!.tagName).toBe("H1");
  });

  it("renders the hero h1 with no chip when the stage is stable", async () => {
    build.stage = "stable";
    await renderLanding();
    expect(heroTitle().querySelector("h1")).not.toBeNull();
    expect(heroTitle().querySelector("[data-release-stage]")).toBeNull();
  });
});

describe("Footer release stage (#841)", () => {
  it("renders the chip as the version link's next sibling", async () => {
    await renderLanding();
    const link = versionLink();
    expect(link).toBeDefined();
    const chip = link.nextElementSibling as HTMLElement;
    expect(chip.getAttribute("data-release-stage")).toBe("alpha");
    expect(chip.textContent).toBe("Alpha");
  });

  it("renders the version link and no chip in the footer when the stage is stable", async () => {
    build.stage = "stable";
    await renderLanding();
    expect(versionLink()).toBeDefined();
    expect(versionLink().nextElementSibling).toBeNull();
    expect(document.querySelector("footer [data-release-stage]")).toBeNull();
  });
});
