import {useEffect} from "react";
import DeckGL from "@deck.gl/react";
import {MapView} from "@deck.gl/core";
import type {Layer, MapViewState, PickingInfo} from "@deck.gl/core";
import {Map as MapLibre} from "react-map-gl/maplibre";
import "maplibre-gl/dist/maplibre-gl.css";
import {useTheme} from "@ois/ui";
import {TriangleAlert} from "lucide-react";

import {CARTO_STYLE, US_HOME} from "./lib/constants";
import {ensureAeroway, type StyleMap} from "./lib/aeroway";
import {useWebglAvailable} from "./hooks/useWebglAvailable";

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

function DefaultFallback() {
  return (
    <div className="flex h-full w-full flex-col items-center justify-center gap-3 px-6 py-16 text-center">
      <TriangleAlert className="h-8 w-8 text-muted-foreground" />
      <div className="text-lg font-semibold">Map can&apos;t be drawn here</div>
      <p className="max-w-md text-sm text-muted-foreground">
        This map needs WebGL, which this browser has disabled. On iPhone and iPad this is almost always{" "}
        <span className="font-medium text-foreground">Lockdown Mode</span> — it turns WebGL off, so the
        map paints black.
      </p>
      <p className="max-w-md text-sm text-muted-foreground">
        To view it, turn Lockdown Mode off for this site: tap{" "}
        <span className="font-medium text-foreground">ᴀA</span> in Safari&apos;s address bar →{" "}
        <span className="font-medium text-foreground">Website Settings</span> →{" "}
        <span className="font-medium text-foreground">Lockdown Mode → Off</span>, then reload.
      </p>
    </div>
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
  const available = useWebglAvailable();

  // Nudge deck.gl to re-measure once layout has settled (0-sized-at-mount safety).
  useEffect(() => {
    const t = setTimeout(() => window.dispatchEvent(new Event("resize")), 150);
    return () => clearTimeout(t);
  }, []);

  if (!available) {
    return <div className={className}>{fallback ?? <DefaultFallback />}</div>;
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
        getTooltip={getTooltip as never}
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
    </div>
  );
}
