// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";

import {DownloadPage} from "./download";

vi.mock("@/components/shell/page-meta", () => ({usePageHeader: () => undefined}));

const RELEASE = {
  tag_name: "v1.4.0",
  assets: [
    {name: "OIS_1.4.0_universal.dmg", browser_download_url: "https://example.test/OIS.dmg"},
    {name: "OIS_1.4.0_x64-setup.exe", browser_download_url: "https://example.test/OIS.exe"},
    {name: "OIS_1.4.0_amd64.AppImage", browser_download_url: "https://example.test/OIS.AppImage"},
    {name: "latest.json", browser_download_url: "https://example.test/latest.json"},
  ],
};

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

beforeEach(() => {
  vi.stubGlobal("fetch", vi.fn());
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

describe("DownloadPage", () => {
  it("links each platform's installer from the latest release", async () => {
    vi.mocked(fetch).mockResolvedValue({
      ok: true,
      json: async () => RELEASE,
    } as Response);

    const el = await render();
    const hrefs = [...el.querySelectorAll("a")].map((a) => a.getAttribute("href"));

    expect(hrefs).toContain("https://example.test/OIS.dmg");
    expect(hrefs).toContain("https://example.test/OIS.exe");
    expect(hrefs).toContain("https://example.test/OIS.AppImage");
    // The manifest is machinery for the updater, not something a human downloads.
    expect(hrefs).not.toContain("https://example.test/latest.json");
    expect(el.textContent).toContain("v1.4.0");
  });

  it("falls back to the releases page when GitHub can't be reached", async () => {
    // Offline, or the 60/hr unauthenticated rate limit. The page must not become a dead end.
    vi.mocked(fetch).mockRejectedValue(new Error("offline"));

    const el = await render();
    const hrefs = [...el.querySelectorAll("a")].map((a) => a.getAttribute("href"));

    expect(el.textContent).toContain("Couldn't reach GitHub");
    expect(hrefs.every((href) => href?.startsWith("https://github.com/VATUSA/OIS/releases"))).toBe(
      true,
    );
  });

  it("falls back when GitHub answers with an error status", async () => {
    vi.mocked(fetch).mockResolvedValue({ok: false, status: 403} as Response);

    const el = await render();

    expect(el.textContent).toContain("Couldn't reach GitHub");
  });

  it("still offers every platform when the release has no matching asset", async () => {
    vi.mocked(fetch).mockResolvedValue({
      ok: true,
      json: async () => ({tag_name: "v1.4.0", assets: []}),
    } as Response);

    const el = await render();
    const hrefs = [...el.querySelectorAll("a")].map((a) => a.getAttribute("href"));

    expect(el.textContent).toContain("macOS");
    expect(el.textContent).toContain("Windows");
    expect(el.textContent).toContain("Linux");
    expect(hrefs.every((href) => href?.startsWith("https://github.com/VATUSA/OIS/releases"))).toBe(
      true,
    );
  });
});
