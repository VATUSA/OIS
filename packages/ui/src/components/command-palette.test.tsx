import * as React from "react";
import {renderToStaticMarkup} from "react-dom/server";
import {describe, expect, it} from "vitest";

import {ScopeChips, cycleScope} from "./command-palette";

describe("cycleScope", () => {
  const ids = ["all", "aircraft", "tmis"];

  it("steps forward and back, wrapping at both ends", () => {
    expect(cycleScope(ids, "all", 1)).toBe("aircraft");
    expect(cycleScope(ids, "tmis", 1)).toBe("all");
    expect(cycleScope(ids, "all", -1)).toBe("tmis");
    expect(cycleScope(ids, "aircraft", -1)).toBe("all");
  });

  it("treats an unknown current scope as the first", () => {
    expect(cycleScope(ids, "gone", 1)).toBe("aircraft");
  });
});

describe("ScopeChips", () => {
  const scopes = [
    { id: "all", label: "All" },
    { id: "tmis", label: "TMIs" },
  ];

  it("marks only the active scope as checked", () => {
    const html = renderToStaticMarkup(<ScopeChips scopes={scopes} scope="tmis" />);
    expect(html).toMatch(new RegExp('aria-checked="true"[^>]*>TMIs<'));
    expect(html).toMatch(new RegExp('aria-checked="false"[^>]*>All<'));
  });

  // The search field normally holds focus, so without these a screen-reader user can neither reach
  // the group nor hear the scope change (VATUSA/OIS#311).
  it("makes only the checked chip tabbable, so the group is one tab stop", () => {
    const html = renderToStaticMarkup(<ScopeChips scopes={scopes} scope="tmis" />);
    expect(html).toMatch(new RegExp('tabindex="0"[^>]*>TMIs<'));
    expect(html).toMatch(new RegExp('tabindex="-1"[^>]*>All<'));
  });

  it("announces the active scope in a live region", () => {
    const html = renderToStaticMarkup(<ScopeChips scopes={scopes} scope="tmis" />);
    expect(html).toMatch(new RegExp('aria-live="polite"[^>]*>TMIs scope<'));
  });
});
