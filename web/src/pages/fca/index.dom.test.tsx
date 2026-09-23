// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {beforeAll, describe, expect, it, vi} from "vitest";

const search = vi.hoisted(() => ({current: {} as {fca?: string}}));
const mapProps = vi.hoisted(() => ({current: null as null | Record<string, unknown>}));

vi.mock("@tanstack/react-router", () => ({ useSearch: () => search.current }));
vi.mock("@/components/map/FcaMapView", () => ({
  FcaMapView: (props: Record<string, unknown>) => {
    mapProps.current = props;
    return null;
  },
}));

import {FcaPage} from "./index";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

function render() {
  const root = createRoot(document.createElement("div"));
  act(() => root.render(<FcaPage />));
  act(() => root.unmount());
}

describe("FcaPage (VATUSA/OIS#348 review)", () => {
  // A release/metering notification links to `/ops/fca?fca=<id>`. Nothing read the param, so every
  // click landed on the generic map instead of the FCA it was about.
  it("opens the FCA named in ?fca=", () => {
    search.current = {fca: "f-zdc"};
    render();
    expect(mapProps.current).toMatchObject({initialFcaId: "f-zdc"});
  });

  it("selects nothing when no FCA is named", () => {
    search.current = {};
    render();
    expect(mapProps.current?.initialFcaId).toBeUndefined();
  });
});
