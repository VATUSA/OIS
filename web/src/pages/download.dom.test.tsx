// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";

vi.mock("@/components/shell/page-meta", () => ({usePageHeader: () => undefined}));

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

/** The split-origin production shape: the web app and the API are different hosts (#738). */
const API = "https://api-ois.vzdc.org";

/**
 * Mounts the page as deployed with `OIS_API_URL=apiUrl`. `API_BASE` is read once at module load from
 * the `window.__OIS_API_URL__` that `deploy/40-ois-config.sh` writes, so the module graph is reset and
 * re-imported per case: this drives the real config path rather than a mocked `API_BASE`.
 */
async function render(apiUrl = API) {
  window.__OIS_API_URL__ = apiUrl;
  vi.resetModules();
  const {DownloadPage} = await import("./download");
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
  delete window.__OIS_API_URL__;
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

  // #534 AC2: each row hands over an installer through our own API, which the desktop CSP allows and
  // GitHub cannot rate-limit. Which asset a platform resolves to is `handlers::desktop`'s decision.
  //
  // #738: the href is the *resolved* URL on the API origin. The literal relative path this case used
  // to assert was the bug: in production it resolves against the web host, which serves the SPA shell.
  it("links every platform at the server-side resolver on the API origin", async () => {
    const el = await render();

    expect(hrefs(el)).toEqual(
      expect.arrayContaining([
        `${API}/api/v1/public/desktop/download/macos`,
        `${API}/api/v1/public/desktop/download/windows`,
        `${API}/api/v1/public/desktop/download/linux`,
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
      expect(row.getAttribute("href")?.startsWith(`${API}/api/v1/public/desktop/download/`)).toBe(true);
    }
    // Still reachable, deliberately, for older builds and formats this page doesn't list.
    expect(hrefs(el)).toContain("https://github.com/VATUSA/OIS/releases");
  });

  // #738 AC1: `OIS_API_URL` empty means same-origin ("empty → same-origin", 40-ois-config.sh). The
  // relative path is then right, and must not become `undefined/api/…` or a thrown `new URL`.
  it("stays same-origin when no API base is configured", async () => {
    const el = await render("");

    expect(hrefs(el)).toEqual(
      expect.arrayContaining([
        "/api/v1/public/desktop/download/macos",
        "/api/v1/public/desktop/download/windows",
        "/api/v1/public/desktop/download/linux",
      ]),
    );
  });

  // The typed client strips a trailing slash from its base; the links must land where it does.
  it("does not double the slash when the API base ends in one", async () => {
    const el = await render(`${API}/`);

    expect(hrefs(el)).toContain(`${API}/api/v1/public/desktop/download/macos`);
  });

  // AC5: installers are unsigned on purpose (`release.yml` documents it), so the first launch warns.
  // A user told what to expect clicks through; one who isn't assumes the download is broken.
  it("says what to expect on first launch while installers are unsigned", async () => {
    const el = await render();

    expect(el.textContent).toMatch(/aren't signed yet|unidentified developer/i);
    expect(el.textContent).toContain("Run anyway");
  });
});
