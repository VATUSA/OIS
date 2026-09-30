// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, beforeEach, describe, expect, it, vi} from "vitest";

/**
 * Event banners come through the API as a `blob:` URL (#429).
 *
 * A raw `<img src={banner_image_url}>` is blocked in the bundled desktop app: organisers host banners
 * wherever they like, and the CSP's `img-src` cannot name every host without becoming `https:`. It
 * also sends no credentials, which the endpoint requires. Fetching and handing over an object URL
 * solves both — as long as the URL is released, since it pins its blob until it is.
 */
const get = vi.hoisted(() => vi.fn());
vi.mock("@/lib/api", () => ({ois: {GET: get}, API_BASE: "", DOCS_URL: ""}));

import {EventBanner} from "./event-banner";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

const created: string[] = [];
const revoked: string[] = [];

let root: ReturnType<typeof createRoot> | undefined;
let host: HTMLElement | undefined;
afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
  root = undefined;
  host = undefined;
});
beforeEach(() => {
  created.length = 0;
  revoked.length = 0;
  get.mockReset();
  // jsdom has no object-URL support, and the point of the test is that we release them.
  let n = 0;
  URL.createObjectURL = vi.fn(() => {
    const url = `blob:test/${++n}`;
    created.push(url);
    return url;
  });
  URL.revokeObjectURL = vi.fn((url: string) => {
    revoked.push(url);
  });
});

async function render(node: React.ReactNode) {
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  await act(async () => {
    root!.render(node);
  });
  // The fetch resolves on a microtask; let it land before asserting.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  return host;
}

describe("EventBanner", () => {
  it("renders the fetched image as a blob URL, not the third-party address", async () => {
    get.mockResolvedValue({data: new Blob(["img"], {type: "image/png"})});

    const el = await render(<EventBanner eventId={7} />);

    const img = el.querySelector("img");
    expect(img).not.toBeNull();
    expect(img!.getAttribute("src")).toBe("blob:test/1");
    // Requested by id — the component never sees the upstream URL.
    expect(get).toHaveBeenCalledWith("/api/v1/events/{id}/banner", {
      params: {path: {id: 7}},
      parseAs: "blob",
    });
  });

  it("releases the object URL on unmount, so the blob isn't pinned", async () => {
    get.mockResolvedValue({data: new Blob(["img"], {type: "image/png"})});

    await render(<EventBanner eventId={7} />);
    expect(created).toEqual(["blob:test/1"]);
    expect(revoked).toEqual([]);

    act(() => root!.unmount());
    root = undefined;

    expect(revoked).toEqual(["blob:test/1"]);
  });

  it("shows the fallback when there is no banner, rather than a broken image", async () => {
    get.mockResolvedValue({data: undefined});

    const el = await render(
      <EventBanner eventId={7} fallback={<div data-testid="placeholder" />} />,
    );

    expect(el.querySelector("img")).toBeNull();
    expect(el.querySelector("[data-testid=placeholder]")).not.toBeNull();
  });

  it("survives the request failing", async () => {
    get.mockRejectedValue(new Error("host unreachable"));

    const el = await render(<EventBanner eventId={7} fallback={<div data-testid="ph" />} />);

    expect(el.querySelector("img")).toBeNull();
    expect(el.querySelector("[data-testid=ph]")).not.toBeNull();
  });
});
