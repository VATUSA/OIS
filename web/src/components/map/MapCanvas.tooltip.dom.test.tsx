// @vitest-environment jsdom
//
// VATUSA/OIS#539. The gate this issue asks for, and the thing eight previous hover "fixes" lacked:
// a test that **mounts the map** and asserts a card element exists and is positioned, rather than
// asserting a pure function returned a string.
//
// `web/src/components/map/lib/tooltip.test.ts` hand-rolls a `PickingInfo` and checks `mapTooltip`'s
// output. Every one of those assertions passed throughout the entire period no tooltip was visible,
// because the breakage was in deck.gl's positioning of the card, not in the string. So this test
// drives the seam that was actually broken: deck hands us a hover, and we assert on the DOM.
//
// `DeckGL` itself cannot mount — jsdom has no WebGL — so it is faked at the module boundary, which
// is also what lets the test capture the `onHover` prop and call it.
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

/** The props the real DeckGL was handed, so the test can drive them. */
const deck = vi.hoisted(() => ({current: null as null | Record<string, unknown>}));

vi.mock("@deck.gl/react", () => ({
  default: (props: Record<string, unknown>) => {
    deck.current = props;
    // Children include the MapLibre basemap, which needs no DOM here.
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

/**
 * A card for anything picked, and `null` for an empty pick.
 *
 * Mirrors `lib/tooltip.ts`'s contract rather than returning a card unconditionally — an
 * always-a-card stub would make "the card clears" pass vacuously, which is how a stub quietly
 * becomes the thing under test.
 */
const cardWhenPicked = (info: {object?: unknown}) =>
  info.object ? {html: "<b>AAL100</b>"} : null;

async function mountMap(
  getTooltip: ((info: {object?: unknown}) => {html: string} | null) | undefined = cardWhenPicked,
) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  await act(async () => {
    root.render(<MapCanvas layers={[]} getTooltip={getTooltip as never} />);
  });
  return host;
}

/** Drive deck's `onHover` the way deck would: canvas-relative cursor plus the viewport size. */
async function hover(at: {x: number; y: number; w?: number; h?: number}) {
  const onHover = deck.current?.onHover as ((info: unknown) => void) | undefined;
  expect(onHover, "MapCanvas must hand deck an onHover").toBeTypeOf("function");
  await act(async () => {
    onHover!({
      x: at.x,
      y: at.y,
      viewport: {width: at.w ?? 1000, height: at.h ?? 800},
      object: {callsign: "AAL100"},
      layer: {id: "aircraft"},
    });
  });
}

const card = () => document.querySelector<HTMLElement>("[data-testid='map-tooltip']");

describe("MapCanvas hover card (VATUSA/OIS#539)", () => {
  /**
   * The regression in one assertion. deck 9.4.0 offset the card by `canvasBounds.y`, which is
   * `-mapHeight` because `.deck-widgets-root` is an unstyled zero-height div at the map's bottom
   * edge — so the card landed at a negative `y` inside an `overflow:hidden` box, every time.
   *
   * Hand `getTooltip` to deck again instead of rendering the card and there is no element at all,
   * so this goes red. That is what the previous eight fixes' tests could not do.
   */
  it("renders a card at the cursor when deck reports a hover", async () => {
    await mountMap();
    await hover({x: 300, y: 200});

    const el = card();
    expect(el).not.toBeNull();
    expect(el!.innerHTML).toContain("AAL100");
    // Below and right of the cursor, and critically **positive** — the bug was a large negative y.
    expect(el!.style.left).toBe("312px");
    expect(el!.style.top).toBe("212px");
    expect(el!.style.transform).toBe("translate(0, 0)");
  });

  it("shows no card until something is hovered", async () => {
    await mountMap();

    expect(card()).toBeNull();
  });

  it("clears the card when the cursor leaves everything", async () => {
    await mountMap();
    await hover({x: 300, y: 200});
    expect(card()).not.toBeNull();

    // deck calls onHover with no card to show once the pick is empty.
    const onHover = deck.current?.onHover as (info: unknown) => void;
    await act(async () => {
      onHover({x: 0, y: 0, viewport: {width: 1000, height: 800}, object: null, layer: null});
    });

    expect(card()).toBeNull();
  });

  /**
   * AC3. The container is `overflow-hidden`, so a card extending past an edge is clipped — the same
   * class of failure as the original bug, just at the other end of the map. Near an edge it has to
   * open back toward the cursor.
   */
  it("opens inward near the right edge rather than being clipped", async () => {
    await mountMap();
    await hover({x: 980, y: 200, w: 1000, h: 800});

    const el = card()!;
    expect(el.style.left).toBe("968px");
    expect(el.style.transform).toBe("translate(-100%, 0)");
  });

  it("opens upward near the bottom edge", async () => {
    await mountMap();
    await hover({x: 300, y: 780, w: 1000, h: 800});

    const el = card()!;
    expect(el.style.top).toBe("768px");
    expect(el.style.transform).toBe("translate(0, -100%)");
  });

  it("opens inward on both axes in the bottom-right corner", async () => {
    await mountMap();
    await hover({x: 980, y: 780, w: 1000, h: 800});

    expect(card()!.style.transform).toBe("translate(-100%, -100%)");
  });

  /**
   * AC4. deck's own tooltip sat at `zIndex: 2` inside a wrapper deck gives `zIndex: 0`, which
   * establishes a stacking context — so it could never clear the page toolbars at `z-[500]` and
   * `z-[650]` that callers pass as `children`. Ours is a sibling of those, above them.
   */
  it("stacks above the page toolbars", async () => {
    await mountMap();
    await hover({x: 300, y: 200});

    expect(card()!.className).toContain("z-[700]");
  });

  /**
   * AC6. deck's default `pickingRadius` is 0 and the aircraft glyph is a masked silhouette, so only
   * its thin opaque pixels were hoverable. Asserted as a prop rather than a behaviour: picking is
   * GPU work that jsdom cannot run, and claiming behavioural coverage here would be false.
   */
  it("gives deck a picking radius, so a thin glyph is hoverable", async () => {
    await mountMap();

    expect(deck.current?.pickingRadius).toBe(4);
  });

  /** deck must not be handed `getTooltip` any more, or it would paint its own card off-screen too. */
  it("does not hand getTooltip to deck", async () => {
    await mountMap();

    expect(deck.current?.getTooltip).toBeUndefined();
  });
});
