import {useMemo} from "react";
import type {Layer, MapViewState, PickingInfo} from "@deck.gl/core";
import {useTheme} from "@ois/ui";

import {MapCanvas} from "./MapCanvas";
import {buildBoundaryLayer} from "./layers/boundaries";
import {buildAircraftLayer, buildLabelLayer, type LabelFlags} from "./layers/aircraft";
import {
  buildRingLayer,
  buildRouteOverlayLayer,
  buildSelectedRouteLayer,
  buildSelectedTrackLayer,
  buildTrailLayer,
  buildWaypointLayer,
} from "./layers/replay";
import {aircraftTooltip} from "./lib/tooltip";
import type {NormAircraft, PathDatum, RGB, RouteGeom} from "./lib/types";

export interface TrafficMapProps {
  initialViewState?: MapViewState;

  // Aircraft (the one required data input; everything else is optional overlay).
  aircraft: NormAircraft[];
  getAircraftColor?: (a: NormAircraft) => RGB;
  getAircraftSize?: (a: NormAircraft) => number;
  selectedAircraftId?: string | null;
  labels?: LabelFlags | null;

  // Overlays.
  boundaries?: GeoJSON.FeatureCollection;
  trails?: PathDatum[];
  routeOverlays?: PathDatum[];
  rings?: { data: NormAircraft[]; nm: number } | null;
  selectedTrack?: PathDatum[];
  filedRoute?: RouteGeom | null;

  // Interactions.
  onAircraftClick?: (id: string) => void;

  // Chrome.
  className?: string;
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
  aircraft,
  getAircraftColor,
  getAircraftSize,
  selectedAircraftId,
  labels,
  boundaries,
  trails,
  routeOverlays,
  rings,
  selectedTrack,
  filedRoute,
  onAircraftClick,
  className,
  children,
}: TrafficMapProps) {
  const { resolvedTheme } = useTheme();

  const anyLabel =
    !!labels && (labels.callsign || labels.type || labels.alt || labels.speed);

  const selectedRoutePath = useMemo<PathDatum[]>(
    () => (filedRoute && filedRoute.path.length >= 2 ? [{ path: filedRoute.path }] : []),
    [filedRoute],
  );

  const layers: Layer[] = [];
  if (boundaries) layers.push(buildBoundaryLayer(boundaries, resolvedTheme));
  if (trails?.length) layers.push(buildTrailLayer(trails, resolvedTheme));
  if (routeOverlays?.length) layers.push(buildRouteOverlayLayer(routeOverlays));
  if (rings?.data.length) layers.push(buildRingLayer(rings.data, rings.nm, resolvedTheme));
  if (selectedTrack?.length) layers.push(buildSelectedTrackLayer(selectedTrack));
  if (selectedRoutePath.length) layers.push(buildSelectedRouteLayer(selectedRoutePath));
  layers.push(
    buildAircraftLayer(aircraft, {
      theme: resolvedTheme,
      getColor: getAircraftColor,
      getSize: getAircraftSize,
      highlightKey: selectedAircraftId,
    }),
  );
  if (anyLabel && labels) layers.push(buildLabelLayer(aircraft, labels, resolvedTheme));
  if (filedRoute?.waypoints.length)
    layers.push(buildWaypointLayer(filedRoute.waypoints, resolvedTheme));

  const handleClick = (info: PickingInfo) => {
    if (info.layer?.id === "aircraft") {
      const id = (info.object as NormAircraft | undefined)?.id;
      if (id) onAircraftClick?.(id);
    }
  };

  return (
    <MapCanvas
      className={className}
      initialViewState={initialViewState}
      layers={layers}
      getTooltip={aircraftTooltip(resolvedTheme)}
      onClick={handleClick}
      getCursor={({ isHovering }) => (isHovering ? "pointer" : "grab")}
    >
      {children}
    </MapCanvas>
  );
}
