import {useCallback, useRef, useState} from "react";
import {FlyToInterpolator, WebMercatorViewport} from "@deck.gl/core";
import type {MapViewState} from "@deck.gl/core";

import {US_HOME} from "../lib/constants";

export interface MapCamera {
  viewState: MapViewState;
  onViewStateChange: (e: { viewState: MapViewState }) => void;
  onResize: (size: { width: number; height: number }) => void;
  /** Animate to a point (keeping or setting zoom). */
  flyTo: (target: { longitude: number; latitude: number; zoom?: number }) => void;
  /** Animate to frame a set of [lon, lat] points. */
  fitBounds: (pts: [number, number][], opts?: { padding?: number; maxZoom?: number }) => void;
  /** Return to the default CONUS view. */
  home: () => void;
}

const TRANSITION = {
  transitionDuration: 700,
  transitionInterpolator: new FlyToInterpolator({ speed: 1.6 }),
};

/**
 * Controlled deck.gl camera with imperative `flyTo`/`fitBounds`/`home`. deck owns the controller
 * (MapLibre is a synced child), so `map.flyTo`/`fitBounds` don't work — these drive the camera by
 * transitioning `viewState` with a FlyToInterpolator (fitBounds solves the target with
 * WebMercatorViewport.fitBounds against the live canvas size).
 */
export function useMapCamera(initial: MapViewState = US_HOME): MapCamera {
  const [viewState, setViewState] = useState<MapViewState>(initial);
  const size = useRef({ width: 800, height: 600 });

  const onViewStateChange = useCallback((e: { viewState: MapViewState }) => {
    setViewState(e.viewState);
  }, []);

  const onResize = useCallback((s: { width: number; height: number }) => {
    if (s.width > 0 && s.height > 0) size.current = s;
  }, []);

  const flyTo = useCallback<MapCamera["flyTo"]>((target) => {
    setViewState((vs) => ({ ...vs, ...target, ...TRANSITION }));
  }, []);

  const fitBounds = useCallback<MapCamera["fitBounds"]>((pts, opts) => {
    if (pts.length === 0) return;
    let minLon = Infinity;
    let minLat = Infinity;
    let maxLon = -Infinity;
    let maxLat = -Infinity;
    for (const [lon, lat] of pts) {
      minLon = Math.min(minLon, lon);
      maxLon = Math.max(maxLon, lon);
      minLat = Math.min(minLat, lat);
      maxLat = Math.max(maxLat, lat);
    }
    // A single point (or a degenerate bounds) can't be "fit" — just fly to it.
    if (minLon === maxLon && minLat === maxLat) {
      flyTo({ longitude: minLon, latitude: minLat, zoom: opts?.maxZoom ?? 9 });
      return;
    }
    try {
      const vp = new WebMercatorViewport({ ...viewState, ...size.current });
      const { longitude, latitude, zoom } = vp.fitBounds(
        [
          [minLon, minLat],
          [maxLon, maxLat],
        ],
        { padding: opts?.padding ?? 80 },
      );
      setViewState((vs) => ({
        ...vs,
        longitude,
        latitude,
        zoom: opts?.maxZoom != null ? Math.min(zoom, opts.maxZoom) : zoom,
        ...TRANSITION,
      }));
    } catch {
      // Bounds off the mercator-projectable range — ignore.
    }
  }, [flyTo, viewState]);

  const home = useCallback(() => setViewState({ ...US_HOME, ...TRANSITION }), []);

  return { viewState, onViewStateChange, onResize, flyTo, fitBounds, home };
}
