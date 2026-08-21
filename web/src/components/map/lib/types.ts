/** Shared, data-source-agnostic types for the TrafficMap component. */

/** One aircraft to render, normalized from either the replay clock or the live/historical feed. */
export interface NormAircraft {
  id: string; // replay: session_id · fca: callsign
  callsign: string;
  actype: string;
  dep: string;
  arr: string;
  lat: number;
  lon: number;
  alt: number;
  gs: number;
  heading: number;
  // Optional flight-plan attributes for facility-map color rules (present on the live feed, absent in
  // replay). See TrafficAircraft on the backend.
  star?: string | null;
  wake?: string;
  flightRules?: string;
  filedAlt?: number;
}

/** A polyline in deck order ([lon, lat] pairs). */
export interface PathDatum {
  path: [number, number][];
}

/** A resolved filed route: polyline in [lon, lat] (deck order) + named waypoints. */
export interface RouteGeom {
  path: [number, number][];
  waypoints: { name: string; lat: number; lon: number }[];
}

export type RGBA = [number, number, number, number];
export type RGB = [number, number, number];
