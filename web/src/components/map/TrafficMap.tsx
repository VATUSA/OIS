import {useCallback, useMemo, useRef, useState} from "react";
import type {Layer, MapViewState, PickingInfo} from "@deck.gl/core";
import {useTheme} from "@ois/ui";

import {useSetting} from "@/lib/settings";
import {MapCanvas} from "./MapCanvas";
import {US_HOME} from "./lib/constants";
import {zoomAircraftScale} from "./lib/aircraft-scale";
import {buildBoundaryLayer} from "./layers/boundaries";
import {buildAircraftLayer, buildLabelLayer, type LabelFlags} from "./layers/aircraft";
import {buildFcaLayers, type MapFca} from "./layers/fca";
import {buildAtcHoverLayer, buildAtcLayers, computeAtcAnchors, type AtcData} from "./layers/atc";
import {buildMatchedLayers, type MatchedFlight} from "./layers/matched";
import {buildNamedRouteLayers, type NamedRoute} from "./layers/routes";
import {buildDraftLayers, type DraftLine} from "./layers/draft";
import {
  buildRingLayer,
  buildRouteOverlayLayer,
  buildSelectedRouteLayer,
  buildSelectedTrackLayer,
  buildTrailLayer,
  buildWaypointLayer,
} from "./layers/replay";
import {AtcMarkers} from "./markers/AtcMarkers";
import type {MapCamera} from "./hooks/useMapCamera";
import {mapTooltip} from "./lib/tooltip";
import type {NormAircraft, PathDatum, RGB, RouteGeom} from "./lib/types";

export interface TrafficMapProps {
  initialViewState?: MapViewState;
  /** Controlled camera (enables flyTo/fitBounds/home). When omitted, deck manages the camera. */
  camera?: MapCamera;

  // Aircraft (the one required data input; everything else is optional overlay).
  aircraft: NormAircraft[];
  aircraftStyle?: "silhouette" | "triangle";
  getAircraftColor?: (a: NormAircraft) => RGB;
  getAircraftSize?: (a: NormAircraft) => number;
  selectedAircraftId?: string | null;
  labels?: LabelFlags | null;

  // Overlays.
  boundaries?: GeoJSON.FeatureCollection;
  /** Draw the boundary bolder + lightly filled (facility map's single-facility focus). */
  boundaryEmphasis?: boolean;
  /** Boundary set to shade online ARTCC centers against. Defaults to `boundaries`; pass the full
   *  national set when `boundaries` is narrowed to one facility, so every online center still shades. */
  atcBoundaries?: GeoJSON.FeatureCollection;
  trails?: PathDatum[];
  routeOverlays?: PathDatum[];
  rings?: { data: NormAircraft[]; nm: number } | null;
  selectedTrack?: PathDatum[];
  filedRoute?: RouteGeom | null;
  fcas?: MapFca[];
  selectedFcaId?: string | null;
  atc?: AtcData | null;
  matched?: MatchedFlight[];
  matchedColor?: string | null;
  /** Overview mode: several FCAs' matched traffic at once, each tinted its own color. */
  matchedGroups?: { id: string; color: string; flights: MatchedFlight[] }[];
  namedRoutes?: NamedRoute[];
  selectedRouteId?: string | null;
  labeledRouteIds?: Set<string>;

  // Drawing/editing (FCA builder). When drawMode is set, clicks add/finish vertices and vertex
  // handles are draggable; coordinates are deck [lon, lat].
  draft?: DraftLine | null;
  drawMode?: "draw" | "edit" | null;
  onAddVertex?: (lngLat: [number, number]) => void;
  onMoveVertex?: (index: number, lngLat: [number, number]) => void;
  onFinishDraft?: () => void;

  // Interactions.
  onAircraftClick?: (id: string) => void;
  onFcaClick?: (id: string) => void;
  onMatchedClick?: (callsign: string) => void;

  // Chrome.
  className?: string;
  /** Cursor over empty map (not hovering a pickable object, not drawing). Default "grab". */
  baseCursor?: string;
  /** react-map-gl <Marker> overlays (rich HTML labels) rendered inside the map. */
  mapChildren?: React.ReactNode;
  children?: React.ReactNode;
}

/**
 * The shared deck.gl + MapLibre traffic map. Data-agnostic: it renders whatever normalized aircraft
 * array + overlays it's handed, so both the replay player (per-frame interpolated array) and the
 * live/historical FCA map (feed hook data) drive the same component. Page chrome (scrubbers, sidebars)
 * wraps it via `children`.
 */
export function TrafficMap({
  initialViewState,
  camera,
  aircraft,
  aircraftStyle,
  getAircraftColor,
  getAircraftSize,
  selectedAircraftId,
  labels,
  boundaries,
  boundaryEmphasis,
  atcBoundaries,
  trails,
  routeOverlays,
  rings,
  selectedTrack,
  filedRoute,
  fcas,
  selectedFcaId,
  atc,
  matched,
  matchedColor,
  matchedGroups,
  namedRoutes,
  selectedRouteId,
  labeledRouteIds,
  draft,
  drawMode,
  onAddVertex,
  onMoveVertex,
  onFinishDraft,
  onAircraftClick,
  onFcaClick,
  onMatchedClick,
  className,
  baseCursor = "grab",
  mapChildren,
  children,
}: TrafficMapProps) {
  const { resolvedTheme } = useTheme();
  const dragIndex = useRef<number | null>(null);
  const [draggingVertex, setDraggingVertex] = useState(false);
  const lastClickT = useRef(0);

  // Track the current zoom so aircraft glyphs can scale with it (VATSIM-Radar style). Rounded to a
  // step so panning-induced micro-zooms don't re-render the layers every frame. Controlled maps also
  // re-render via the camera, but tracking it here keeps the one code path for both.
  const dynamicScale = useSetting("map.dynamicAircraftScale", true).value;
  // User-chosen base size (percent, e.g. "80"), independent of the zoom-driven scale above.
  const iconSizePct = useSetting("map.aircraftIconSize", "100").value;
  const [zoom, setZoom] = useState(
    () => initialViewState?.zoom ?? camera?.viewState.zoom ?? US_HOME.zoom,
  );
  const handleViewStateChange = useCallback(
    (e: { viewState: MapViewState }) => {
      camera?.onViewStateChange?.(e);
      const z = e.viewState.zoom;
      setZoom((prev) => (Math.abs(prev - z) >= ZOOM_STEP ? z : prev));
    },
    [camera],
  );
  const sizeScale = (dynamicScale ? zoomAircraftScale(zoom) : 1) * (Number(iconSizePct) / 100);

  const anyLabel =
    !!labels && (labels.callsign || labels.type || labels.alt || labels.speed);

  // Centers shade against the full national set, not the (possibly single-facility) outline set.
  const centerBoundaries = atcBoundaries ?? boundaries;
  const atcAnchors = useMemo(
    () => (atc && centerBoundaries ? computeAtcAnchors(atc, centerBoundaries) : []),
    [atc, centerBoundaries],
  );

  const selectedRoutePath = useMemo<PathDatum[]>(
    () => (filedRoute && filedRoute.path.length >= 2 ? [{ path: filedRoute.path }] : []),
    [filedRoute],
  );

  const layers: Layer[] = [];
  if (boundaries) layers.push(buildBoundaryLayer(boundaries, resolvedTheme, boundaryEmphasis));
  if (atc && centerBoundaries) layers.push(...buildAtcLayers(atc, centerBoundaries));
  if (atcAnchors.length) layers.push(buildAtcHoverLayer(atcAnchors));
  if (trails?.length) layers.push(buildTrailLayer(trails, resolvedTheme));
  if (routeOverlays?.length) layers.push(buildRouteOverlayLayer(routeOverlays));
  if (namedRoutes?.length)
    layers.push(...buildNamedRouteLayers(namedRoutes, selectedRouteId, labeledRouteIds ?? EMPTY_SET));
  if (rings?.data.length) layers.push(buildRingLayer(rings.data, rings.nm, resolvedTheme));
  if (fcas?.length) layers.push(...buildFcaLayers(fcas, selectedFcaId));
  if (matched?.length && matchedColor)
    layers.push(...buildMatchedLayers(matched, matchedColor, aircraftStyle ?? "silhouette", sizeScale));
  for (const group of matchedGroups ?? [])
    if (group.flights.length)
      layers.push(
        ...buildMatchedLayers(group.flights, group.color, aircraftStyle ?? "silhouette", sizeScale, `-${group.id}`),
      );
  if (selectedTrack?.length) layers.push(buildSelectedTrackLayer(selectedTrack));
  if (selectedRoutePath.length) layers.push(buildSelectedRouteLayer(selectedRoutePath));
  layers.push(
    buildAircraftLayer(aircraft, {
      theme: resolvedTheme,
      style: aircraftStyle,
      getColor: getAircraftColor,
      getSize: getAircraftSize,
      sizeScale,
      highlightKey: selectedAircraftId,
    }),
  );
  if (anyLabel && labels) layers.push(buildLabelLayer(aircraft, labels, resolvedTheme));
  if (filedRoute?.waypoints.length)
    layers.push(buildWaypointLayer(filedRoute.waypoints, resolvedTheme));
  if (draft) layers.push(...buildDraftLayers(draft));

  const handleClick = (info: PickingInfo, event: unknown) => {
    if (drawMode === "draw") {
      // Detect a double-click (deck has no onDblClick; doubleClickZoom is off while drawing).
      const t = (event as { srcEvent?: { timeStamp?: number } })?.srcEvent?.timeStamp ?? performance.now();
      const isDouble = t - lastClickT.current < 300;
      lastClickT.current = t;
      if (isDouble) {
        onFinishDraft?.();
        return;
      }
      // A click on a vertex handle (to grab it) shouldn't also add a point.
      if (info.layer?.id !== "draft-vertices" && info.coordinate) {
        onAddVertex?.(info.coordinate as [number, number]);
      }
      return;
    }
    if (info.layer?.id === "aircraft") {
      const id = (info.object as NormAircraft | undefined)?.id;
      if (id) onAircraftClick?.(id);
    } else if (info.layer?.id?.startsWith("matched")) {
      // The pickable glyph layer is "matched" (single) or "matched-<fcaId>" (overview groups);
      // its sibling trail/dot/badge layers aren't pickable, so a pick here is always a glyph.
      const cs = (info.object as MatchedFlight | undefined)?.callsign;
      if (cs) onMatchedClick?.(cs);
    } else if (info.layer?.id === "fca-lines") {
      const id = (info.object as { id: string } | undefined)?.id;
      if (id) onFcaClick?.(id);
    }
  };

  const handleDragStart = (info: PickingInfo, event: unknown) => {
    if (drawMode && info.layer?.id === "draft-vertices" && info.index != null && info.index >= 0) {
      dragIndex.current = info.index;
      setDraggingVertex(true);
      (event as { stopPropagation?: () => void })?.stopPropagation?.();
    }
  };
  const handleDrag = (info: PickingInfo) => {
    if (dragIndex.current != null && info.coordinate) {
      onMoveVertex?.(dragIndex.current, info.coordinate as [number, number]);
    }
  };
  const handleDragEnd = () => {
    if (dragIndex.current != null) {
      dragIndex.current = null;
      setDraggingVertex(false);
    }
  };

  // While drawing, disable double-click-zoom (dbl-click finishes the line); while a vertex is being
  // dragged, disable panning so the map holds still.
  const controller = drawMode
    ? { doubleClickZoom: false, dragPan: !draggingVertex, dragRotate: false }
    : true;

  return (
    <MapCanvas
      className={className}
      initialViewState={initialViewState}
      viewState={camera?.viewState}
      onViewStateChange={handleViewStateChange}
      onResize={camera?.onResize}
      controller={controller}
      layers={layers}
      getTooltip={mapTooltip(resolvedTheme)}
      onClick={handleClick}
      onDragStart={handleDragStart}
      onDrag={handleDrag}
      onDragEnd={handleDragEnd}
      getCursor={({ isHovering }) => (drawMode === "draw" ? "crosshair" : isHovering ? "pointer" : baseCursor)}
      mapChildren={
        <>
          {atcAnchors.length > 0 && <AtcMarkers anchors={atcAnchors} />}
          {mapChildren}
        </>
      }
    >
      {children}
    </MapCanvas>
  );
}

const EMPTY_SET: Set<string> = new Set();

/** Re-scale glyphs only when zoom moves at least this much, so panning doesn't churn the layers. */
const ZOOM_STEP = 0.1;
