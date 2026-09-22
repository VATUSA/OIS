import {describe, expect, it} from "vitest";

import {API_BASE, ois} from "@/lib/api";

describe("no network in tests (VATUSA/OIS#387)", () => {
  // The shared client is the thing that mattered: it captured `fetch` at import, so only a stub in
  // place before that import is ever seen. This fails if the setup file is removed or unwired.
  it("refuses the shared API client's requests before they leave the process", async () => {
    await expect(
      ois.GET("/api/v1/me/preferences/{namespace}", { params: { path: { namespace: "favorites" } } }),
    ).rejects.toThrow(/network disabled in tests/);
  });

  it("refuses a plain fetch to API_BASE too", async () => {
    await expect(fetch(`${API_BASE}/health`)).rejects.toThrow(/network disabled in tests/);
  });
});
