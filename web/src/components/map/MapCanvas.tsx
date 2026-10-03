import {useEffect, useState} from "react";
import DeckGL from "@deck.gl/react";
import {MapView} from "@deck.gl/core";
import type {Layer, MapViewState, PickingInfo} from "@deck.gl/core";
import {Map as MapLibre} from "react-map-gl/maplibre";
import "maplibre-gl/dist/maplibre-gl.css";
import {useTheme} from "@ois/ui";

import {CARTO_STYLE, US_HOME} from "./lib/constants";
import {ensureAeroway, type StyleMap} from "./lib/aeroway";
import {useWebglAvailable} from "./hooks/useWebglAvailable";
import {MapFallback} from "./MapFallback";

/** deck renders every visible world copy (matching MapLibre's renderWorldCopies) — replaces the old
 * Leaflet longitude-offset machinery. */
const MAP_VIEW = new MapView({ repeat: true });

/**
 * maplibre-gl 6 resolves its worker script's URL by string-concatenating a filename at runtime
 * (`new URL('./' + name, import.meta.url)`), which neither Vite's dev optimizer nor its production
 * Rollup build can statically detect as a worker import — the real worker file never gets
 * bundled/served, so the request silently falls back to `index.html` and every vector tile fails
 * to parse (the basemap stays blank; deck.gl, which has no worker dependency, is unaffected).
 * Loading the worker ourselves via Vite's `?worker&url` suffix (a static, analyzable specifier)
 * makes Vite bundle it — resolving its own relative imports — into a self-contained asset and hand
 * back its URL; setting `config.WORKER_URL` before react-map-gl constructs the map makes maplibre
 * use that instead of computing its own. Both imports stay dynamic so maplibre-gl remains in its
 * own lazy chunk rather than bloating the main bundle.
 */
const mapLibPromise = Promise.all([
  import("maplibre-gl"),
  import("maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url"),
]).then(([mod, workerUrl]) => {
  mod.setWorkerUrl(workerUrl.default);
  return mod;
});

/** deck.gl calls props.onResize directly, so an explicit `undefined` (no camera) crashes it. */
const NOOP = () => {};

type TooltipFn = (info: PickingInfo) => { html: string; style?: Record<string, string> } | null;
type EventHandler = (info: PickingInfo, event: unknown) => void;

interface MapCanvasProps {
  layers: Layer[];
  /** Uncontrolled initial camera (deck manages it). */
  initialViewState?: MapViewState;
  /** Controlled camera (used for programmatic flyTo/fitBounds). */
  viewState?: MapViewState;
  onViewStateChange?: (e: { viewState: MapViewState }) => void;
  controller?: boolean | object;
  getTooltip?: TooltipFn;
  onClick?: EventHandler;
  onDragStart?: EventHandler;
  onDrag?: EventHandler;
  onDragEnd?: EventHandler;
  onResize?: (size: { width: number; height: number }) => void;
  getCursor?: (state: { isDragging: boolean; isHovering: boolean }) => string;
  /** react-map-gl <Marker> overlays rendered inside the MapLibre map. */
  mapChildren?: React.ReactNode;
  /** HTML controls positioned over the canvas (the caller positions them). */
  children?: React.ReactNode;
  className?: string;
  fallback?: React.ReactNode;
}

/** A hover card: the HTML `getTooltip` produced, plus where the cursor was. */
type HoverCard = {
  html: string;
  style?: Record<string, string>;
  /** Cursor position in canvas pixels, and the canvas size, both straight from deck's `info`. */
  x: number;
  y: number;
  width: number;
  height: number;
};

/** Gap between the cursor and the card's nearest corner. */
const CARD_GAP = 12;
/**
 * The largest a card is assumed to get, used only to decide *which side* of the cursor it opens on.
 *
 * The flip itself is exact — `translate(-100%)` uses the card's real width — so these only need to be
 * roughly right: too small and a card near the edge opens outward and gets clipped, too large and it
 * flips inward a little early. Measuring the card instead would mean rendering it, measuring, then
 * moving it, which is a visible jump.
 */
const CARD_MAX = { width: 280, height: 160 };

/**
 * Where the card sits, and which way it opens.
 *
 * The container is `overflow-hidden` (see `className`'s default), so a card that would extend past an
 * edge is not merely ugly — it is cut off, which is the whole bug this is fixing. Near the right or
 * bottom edge the card therefore opens back toward the cursor instead.
 */
function cardPlacement(card: HoverCard) {
  const flipX = card.x + CARD_GAP + CARD_MAX.width > card.width;
  const flipY = card.y + CARD_GAP + CARD_MAX.height > card.height;
  return {
    left: card.x + (flipX ? -CARD_GAP : CARD_GAP),
    top: card.y + (flipY ? -CARD_GAP : CARD_GAP),
    transform: `translate(${flipX ? "-100%" : "0"}, ${flipY ? "-100%" : "0"})`,
  };
}

/**
 * The hover card, rendered by us rather than by deck.gl's tooltip widget (#539).
 *
 * deck 9.4.0's widget offsets the card by `getCanvasBounds()`, which measures against
 * `.deck-widgets-root` — a div `@deck.gl/react` creates with no style at all, so it is a zero-height
 * block sitting at the *bottom* of the map. Every card was therefore translated a full map-height
 * upward inside an `overflow: hidden` box and was never visible, which is why eight successive fixes
 * to `getTooltip`'s *content* changed nothing.
 *
 * Owning the element also puts it above the page toolbars (`z-[500]`/`z-[650]`), which deck's widget
 * could not be: deck's wrapper sets `zIndex: 0`, establishing a stacking context that traps its
 * tooltip container at `zIndex: 2` underneath them.
 *
 * `dangerouslySetInnerHTML` is the same trust boundary deck used (`el.innerHTML = displayInfo.html`),
 * not a new one — `lib/tooltip.ts` escapes every interpolated value through its `esc()`.
 */
function MapTooltip({ card }: { card: HoverCard }) {
  const { left, top, transform } = cardPlacement(card);
  return (
    <div
      data-testid="map-tooltip"
      // Above the toolbars the callers pass as `children`, and never a pointer target itself —
      // a card under the cursor would otherwise steal the next hover and flicker.
      className="pointer-events-none absolute z-[700] max-w-[280px] rounded-md border border-line bg-panel-2 px-2 py-1.5 text-xs text-ink shadow-none"
      style={{ left, top, transform, ...card.style }}
      dangerouslySetInnerHTML={{ __html: card.html }}
    />
  );
}

/**
 * The dumb map shell: DeckGL (world-copy-repeating) with a MapLibre CARTO vector basemap + the aeroway
 * airport-layout overlay + a WebGL2-unavailable fallback. Knows nothing about aircraft/FCAs — it just
 * renders the `layers` it's given, forwards picking/drag events, and hosts overlay children.
 */
export function MapCanvas({
  layers,
  initialViewState,
  viewState,
  onViewStateChange,
  controller = true,
  getTooltip,
  onClick,
  onDragStart,
  onDrag,
  onDragEnd,
  onResize,
  getCursor,
  mapChildren,
  children,
  className = "relative h-full w-full overflow-hidden",
  fallback,
}: MapCanvasProps) {
  const { resolvedTheme } = useTheme();
  const { ok: webglOk, retry } = useWebglAvailable();
  const [hovered, setHovered] = useState<HoverCard | null>(null);

  // Nudge deck.gl to re-measure once layout has settled (0-sized-at-mount safety).
  useEffect(() => {
    const t = setTimeout(() => window.dispatchEvent(new Event("resize")), 150);
    return () => clearTimeout(t);
  }, []);

  if (!webglOk) {
    return <div className={className}>{fallback ?? <MapFallback onRetry={retry} />}</div>;
  }

  const controlled = viewState != null;

  return (
    <div className={className}>
      <DeckGL
        views={MAP_VIEW}
        {...(controlled
          ? { viewState }
          : { initialViewState: initialViewState ?? US_HOME })}
        // Always forward view changes: controlled maps feed viewState back through it, and uncontrolled
        // maps (the replay player) still need it so the caller can track zoom for icon scaling.
        onViewStateChange={(onViewStateChange ?? NOOP) as never}
        controller={controller}
        layers={layers}
        // NOT `getTooltip`: deck 9.4.0 paints that card off-screen (see `MapTooltip`). The same
        // function is still the source of the content — only who positions it has changed, so every
        // caller keeps passing `getTooltip` exactly as before.
        onHover={((info: PickingInfo) => {
          const card = getTooltip?.(info) ?? null;
          setHovered(
            card
              ? {
                  ...card,
                  x: info.x,
                  y: info.y,
                  // From deck rather than the DOM: no layout read on every pointermove, and a test
                  // can set it.
                  width: info.viewport?.width ?? 0,
                  height: info.viewport?.height ?? 0,
                }
              : null,
          );
        }) as never}
        // deck's default is 0, and the aircraft glyph is a 48x48 masked silhouette drawn at ~26px —
        // so without this only the thin aeroplane-shaped opaque pixels are hoverable at all (#539).
        pickingRadius={4}
        onClick={onClick as never}
        onDragStart={onDragStart as never}
        onDrag={onDrag as never}
        onDragEnd={onDragEnd as never}
        onResize={onResize ?? NOOP}
        getCursor={getCursor}
        style={{ position: "absolute", top: "0", left: "0", width: "100%", height: "100%" }}
      >
        <MapLibre
          // Remount on theme change: react-map-gl's style diff doesn't reliably swap between the two
          // vendored basemaps once our aeroway layers are added, so force a fresh basemap. deck owns
          // the camera, so MapLibre re-syncs to the current view with no reset.
          key={resolvedTheme}
          mapLib={mapLibPromise}
          mapStyle={CARTO_STYLE[resolvedTheme]}
          attributionControl={false}
          // deck.gl renders this as a sibling div ON TOP of its own canvas (same absolute bounds),
          // and neither deck.gl nor react-map-gl sets pointer-events on it — without this, the
          // basemap div/canvas silently swallows every click/drag before deck's own canvas (where
          // picking + the pan/zoom controller live) ever sees it. No native MapLibre control is used
          // anywhere in this app, so nothing inside needs to stay clickable.
          style={{ pointerEvents: "none" }}
          onLoad={(e) => ensureAeroway(e.target as unknown as StyleMap, resolvedTheme)}
          onStyleData={(e) => ensureAeroway(e.target as unknown as StyleMap, resolvedTheme)}
        >
          {mapChildren}
        </MapLibre>
      </DeckGL>
      {children}
      {hovered && <MapTooltip card={hovered} />}
    </div>
  );
}
