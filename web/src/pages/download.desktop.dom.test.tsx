// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, describe, expect, it, vi} from "vitest";

import {DownloadPage} from "./download";

const env = vi.hoisted(() => ({desktop: false}));
vi.mock("@/lib/platform", () => ({isTauri: () => env.desktop}));
vi.mock("@/components/shell/page-meta", () => ({usePageHeader: () => undefined}));

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

async function render(desktop: boolean) {
  env.desktop = desktop;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => {
    root.render(<DownloadPage />);
  });
  return container;
}

const links = (el: HTMLElement) => [...el.querySelectorAll("a")];
const platformRows = (el: HTMLElement) =>
  links(el).filter((a) => /macOS|Windows|Linux/.test(a.textContent ?? ""));

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

/**
 * VATUSA/OIS#751. The desktop app renders this same page (its sidebar links `/download`, #534), where a
 * platform row is a bare link that would navigate the main window off-app to the installer redirect.
 * The user already has the app, and it updates itself, so the rows are not offered there at all.
 */
describe("DownloadPage inside the desktop app", () => {
  it("offers no platform download, so nothing can navigate the window off-app", async () => {
    const el = await render(true);

    expect(platformRows(el)).toHaveLength(0);
    expect(links(el).some((a) => a.getAttribute("href")?.includes("/desktop/download/"))).toBe(false);
    expect(el.textContent).toMatch(/already running the desktop app/i);
    // The releases page stays reachable, deliberately, as it is on the web.
    expect(links(el).map((a) => a.getAttribute("href"))).toContain(
      "https://github.com/VATUSA/OIS/releases",
    );
  });

  // Positive control: the same mount in a browser still lists every platform, so the case above
  // cannot pass on a page that renders nothing.
  it("still lists every platform in a browser", async () => {
    const el = await render(false);

    expect(platformRows(el)).toHaveLength(3);
    expect(el.textContent).not.toMatch(/already running the desktop app/i);
  });
});
