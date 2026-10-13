// @vitest-environment jsdom
//
// VATUSA/OIS#821. deck builds its props as `{...defaultProps, ...props}` and calls
// `props.getCursor` on every render without a null check. MapCanvas used to forward
// `getCursor={getCursor}` unconditionally, so a caller that set no cursor (the admin sector map)
// handed deck an own `getCursor: undefined`, which replaced deck's default; deck threw and the
// overlay drew nothing. `DeckGL` cannot mount in jsdom (no WebGL), so it is faked at the module
// boundary to capture the props, and they are merged over deck's real defaults the way deck does.
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {Deck} from "@deck.gl/core";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

const deck = vi.hoisted(() => ({current: null as null | Record<string, unknown>}));

vi.mock("@deck.gl/react", () => ({
  default: (props: Record<string, unknown>) => {
    deck.current = props;
    return null;
  },
}));
vi.mock("react-map-gl/maplibre", () => ({Map: () => null}));
vi.mock("maplibre-gl/dist/maplibre-gl.css", () => ({}));
vi.mock("@ois/ui", () => ({useTheme: () => ({resolvedTheme: "dark"})}));
vi.mock("./hooks/useWebglAvailable", () => ({useWebglAvailable: () => ({ok: true, retry: () => {}})}));
vi.mock("./MapFallback", () => ({MapFallback: () => null}));
vi.mock("./lib/aeroway", () => ({ensureAeroway: () => {}}));

import {MapCanvas} from "./MapCanvas";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

const roots: {root: ReturnType<typeof createRoot>; host: HTMLElement}[] = [];
afterEach(() => {
  for (const {root, host} of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
  deck.current = null;
});

async function mount(element: React.ReactElement) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  await act(async () => {
    root.render(element);
  });
}

/** The props deck would run with: what MapCanvas handed it, spread over deck's own defaults. */
function deckProps() {
  expect(deck.current, "MapCanvas never rendered DeckGL").not.toBeNull();
  return {...Deck.defaultProps, ...deck.current} as Record<string, unknown>;
}

const cursorState = {isDragging: false, isHovering: false};

describe("MapCanvas getCursor", () => {
  it("leaves deck's default cursor in place when the caller sets none", async () => {
    await mount(<MapCanvas layers={[]} />);

    const getCursor = deckProps().getCursor as (s: typeof cursorState) => string;
    expect(getCursor).toBeTypeOf("function");
    // deck's default, called the way deck calls it.
    expect(getCursor(cursorState)).toBe("grab");
    expect(getCursor({...cursorState, isDragging: true})).toBe("grabbing");
  });

  it("hands deck the caller's cursor when it sets one", async () => {
    const custom = vi.fn(() => "crosshair");
    await mount(<MapCanvas layers={[]} getCursor={custom} />);

    const getCursor = deckProps().getCursor as (s: typeof cursorState) => string;
    expect(getCursor(cursorState)).toBe("crosshair");
    expect(custom).toHaveBeenCalledWith(cursorState);
  });
});
