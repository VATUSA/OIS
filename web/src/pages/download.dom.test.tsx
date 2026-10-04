// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";

import {DownloadPage} from "./download";

vi.mock("@/components/shell/page-meta", () => ({usePageHeader: () => undefined}));

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

async function render() {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => {
    root.render(<DownloadPage />);
  });
  return container;
}

const hrefs = (el: HTMLElement) => [...el.querySelectorAll("a")].map((a) => a.getAttribute("href"));

beforeEach(() => {
  // Stubbed so a stray call would be observable, not so it can be driven. See the first case.
  vi.stubGlobal("fetch", vi.fn());
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

/**
 * These cases replaced a set built around a browser-side GitHub call (#534).
 *
 * The page used to `fetch` `api.github.com`, match assets by filename suffix, and fall back to the
 * releases page when that failed — which it did inside the desktop app (the host is absent from the
 * CSP `connect-src`) and whenever GitHub's 60/hr unauthenticated limit was spent. The old tests
 * asserted that fallback, so they documented the defect rather than the fix; replacing them is the
 * point of ACs 2–4, not collateral.
 */
describe("DownloadPage", () => {
  /**
   * AC3, in its strongest available form. Asserting "no browser-side call to api.github.com" by
   * checking that `fetch` is never touched is stronger than any assertion about what a mocked
   * response produces: it shows the dependency is *gone*, not merely handled.
   */
  it("makes no network call at all", async () => {
    await render();

    expect(fetch).not.toHaveBeenCalled();
  });

  // AC2: each row hands over an installer through our own origin, which the desktop CSP allows and
  // GitHub cannot rate-limit. Which asset a platform resolves to is `handlers::desktop`'s decision.
  it("links every platform at the server-side resolver", async () => {
    const el = await render();

    expect(hrefs(el)).toEqual(
      expect.arrayContaining([
        "/api/v1/public/desktop/download/macos",
        "/api/v1/public/desktop/download/windows",
        "/api/v1/public/desktop/download/linux",
      ]),
    );
    expect(el.textContent).toContain("macOS");
    expect(el.textContent).toContain("Windows");
    expect(el.textContent).toContain("Linux");
  });

  /**
   * AC4. The dishonesty this issue was filed about was not that a fallback existed — it was that
   * every *platform row* silently became a link to the releases page while still looking like a
   * direct download. The releases link stays, but only as its own labelled link.
   */
  it("never points a platform row at the GitHub releases page", async () => {
    const el = await render();
    const rows = [...el.querySelectorAll("a")].filter((a) =>
      /macOS|Windows|Linux/.test(a.textContent ?? ""),
    );

    expect(rows).toHaveLength(3);
    for (const row of rows) {
      expect(row.getAttribute("href")).toMatch(/^\/api\/v1\/public\/desktop\/download\//);
    }
    // Still reachable, deliberately, for older builds and formats this page doesn't list.
    expect(hrefs(el)).toContain("https://github.com/VATUSA/OIS/releases");
  });

  // AC5: installers are unsigned on purpose (`release.yml` documents it), so the first launch warns.
  // A user told what to expect clicks through; one who isn't assumes the download is broken.
  it("says what to expect on first launch while installers are unsigned", async () => {
    const el = await render();

    expect(el.textContent).toMatch(/aren't signed yet|unidentified developer/i);
    expect(el.textContent).toContain("Run anyway");
  });
});
