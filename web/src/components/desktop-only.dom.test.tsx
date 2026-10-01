// @vitest-environment jsdom
import {renderToStaticMarkup} from "react-dom/server";
import {afterEach, describe, expect, it} from "vitest";

import {DesktopOnly} from "./desktop-only";

afterEach(() => {
  delete window.__TAURI_INTERNALS__;
});

describe("DesktopOnly", () => {
  it("renders nothing on the web build", () => {
    const html = renderToStaticMarkup(
      <DesktopOnly>
        <button>Pop out</button>
      </DesktopOnly>,
    );
    expect(html).toBe("");
  });

  it("renders its children in the desktop shell", () => {
    window.__TAURI_INTERNALS__ = {};
    const html = renderToStaticMarkup(
      <DesktopOnly>
        <button>Pop out</button>
      </DesktopOnly>,
    );
    expect(html).toContain("Pop out");
  });

  it("renders the fallback on web when one is given", () => {
    const html = renderToStaticMarkup(
      <DesktopOnly fallback={<span>Available in the desktop app</span>}>
        <button>Pop out</button>
      </DesktopOnly>,
    );
    expect(html).toContain("Available in the desktop app");
    expect(html).not.toContain("Pop out");
  });
});
