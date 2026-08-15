import {useCallback, useEffect, useMemo, useRef, useState} from "react";
import L from "leaflet";
import "leaflet/dist/leaflet.css";
import {Button, ConfirmButton, Input, useTheme} from "@ois/ui";
import {ChevronDown, Maximize2, Minus, Pencil, Plus, RefreshCw, Tag, Trash2, X,} from "lucide-react";

import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {
  type AircraftRoute,
  type Fca,
  type FcaFlight,
  toUpsert,
  type UpsertFca,
  useAircraftRoute,
  useCreateFca,
  useDataStatus,
  useDeleteFca,
  useFcaCounts,
  useFcas,
  useFcaTraffic,
  useRefreshData,
  useRouteCoverage,
  useTraffic,
  useUpdateFca,
} from "@/lib/fca";
import {type MapRoute, type UpsertRoute, useCreateRoute, useDeleteRoute, useRoutes, useUpdateRoute,} from "@/lib/route";
import {FcaDetail} from "@/pages/fca/detail";
import boundariesGeo from "@/assets/artcc-boundaries.json";

const FCA_COLORS = [
  "#f59e0b",
  "#ec4899",
  "#84cc16",
  "#f97316",
  "#38bdf8",
  "#f87171",
];

type LatLng = [number, number];
type Phase = "draw" | "edit";

type Draft = {
  id: string | null;
  name: string;
  color: string;
  artcc: string;
  points: LatLng[];
  dests: string;
  origins: string;
  fixes: string;
  scope: string;
  minFl: string;
  maxFl: string;
  mode: "rate" | "mit";
  rate: number;
  mit: number;
};

const list = (a: string[]) => a.join(" ");
const parseList = (s: string) =>
  s
    .split(/[\s,]+/)
    .filter(Boolean)
    .map((t) => t.toUpperCase());
const parseFl = (s: string): number | null => {
  const n = parseInt(s.trim(), 10);
  return Number.isFinite(n) ? n : null;
};

/** Age in whole days of a `YYYY-MM-DD` NASR cycle date, or null if unparseable. */
function cycleAgeDays(cycle: string): number | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(cycle);
  if (!m) return null;
  const d = Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
  return Math.floor((Date.now() - d) / 86_400_000);
}

/** CARTO basemaps + map background per theme. */
const CARTO = {
  dark: "https://{s}.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}{r}.png",
  light: "https://{s}.basemaps.cartocdn.com/light_all/{z}/{x}/{y}{r}.png",
} as const;
const MAP_BG = { dark: "#0a0a0a", light: "#e5e7eb" } as const;

function haversine(a: LatLng, b: LatLng): number {
  const R = 3440.065;
  const dLat = ((b[0] - a[0]) * Math.PI) / 180;
  const dLon = ((b[1] - a[1]) * Math.PI) / 180;
  const la1 = (a[0] * Math.PI) / 180;
  const la2 = (b[0] * Math.PI) / 180;
  const h =
    Math.sin(dLat / 2) ** 2 +
    Math.cos(la1) * Math.cos(la2) * Math.sin(dLon / 2) ** 2;
  return 2 * R * Math.asin(Math.sqrt(h));
}
function lineNm(pts: LatLng[]): number {
  let d = 0;
  for (let i = 0; i < pts.length - 1; i++) d += haversine(pts[i], pts[i + 1]);
  return d;
}
/** Make a polyline's longitudes continuous so it draws the *short* way across the
 *  antimeridian (Pacific) instead of wrapping the long way around the whole map. Each point's
 *  longitude is shifted by ±360 to stay within 180° of the previous one; the resulting
 *  longitudes may exceed ±180, which Leaflet renders correctly on the wrapped world copies. */
function unwrapLng(pts: LatLng[]): LatLng[] {
  if (pts.length === 0) return pts;
  const out: LatLng[] = [pts[0]];
  for (let i = 1; i < pts.length; i++) {
    const prev = out[i - 1][1];
    let lng = pts[i][1];
    while (lng - prev > 180) lng -= 360;
    while (lng - prev < -180) lng += 360;
    out.push([pts[i][0], lng]);
  }
  return out;
}

/** The longitude offsets (multiples of 360°) that cover the current horizontal view, so
 *  overlays can be drawn on every visible copy of the world — like the repeating tiles.
 *  Capped so an extreme zoom-out can't spawn a runaway number of copies. */
function copyOffsets(map: L.Map): number[] {
  const b = map.getBounds();
  const start = Math.floor(b.getWest() / 360);
  const end = Math.ceil(b.getEast() / 360);
  const out: number[] = [];
  for (let i = start; i <= end && out.length < 9; i++) out.push(i * 360);
  return out.length ? out : [0];
}
/** Shift every vertex's longitude by `off` (a multiple of 360°) to place it on another copy. */
const shiftLine = (pts: LatLng[], off: number): LatLng[] =>
  off ? pts.map((p) => [p[0], p[1] + off] as LatLng) : pts;
/** Normalize each vertex's longitude back into [-180, 180) for storage. */
const normPoints = (pts: LatLng[]): LatLng[] =>
  pts.map(([lat, lng]) => [lat, ((((lng + 180) % 360) + 360) % 360) - 180]);

/** The point halfway along a polyline by arc length (the true visual center). */
function midpointOf(pts: LatLng[]): LatLng {
  if (pts.length < 2) return pts[0];
  const segs = pts.slice(1).map((p, i) => haversine(pts[i], p));
  let half = segs.reduce((a, b) => a + b, 0) / 2;
  for (let i = 0; i < segs.length; i++) {
    if (half <= segs[i]) {
      const f = segs[i] ? half / segs[i] : 0;
      return [
        pts[i][0] + (pts[i + 1][0] - pts[i][0]) * f,
        pts[i][1] + (pts[i + 1][1] - pts[i][1]) * f,
      ];
    }
    half -= segs[i];
  }
  return pts[pts.length - 1];
}

function blankDraft(count: number): Draft {
  return {
    id: null,
    name: `FCA ${count + 1}`,
    color: FCA_COLORS[count % FCA_COLORS.length],
    artcc: "",
    points: [],
    dests: "",
    origins: "",
    fixes: "",
    scope: "",
    minFl: "",
    maxFl: "",
    mode: "rate",
    rate: 30,
    mit: 15,
  };
}
function draftFrom(fca: Fca): Draft {
  return {
    id: fca.id,
    name: fca.name,
    color: fca.color,
    artcc: fca.artcc,
    points: (fca.points as LatLng[]) ?? [],
    dests: list(fca.dests),
    origins: list(fca.origins),
    fixes: list(fca.fixes),
    scope: list(fca.scope),
    minFl: fca.min_fl != null ? String(fca.min_fl) : "",
    maxFl: fca.max_fl != null ? String(fca.max_fl) : "",
    mode: fca.mode === "mit" ? "mit" : "rate",
    rate: fca.rate,
    mit: fca.mit,
  };
}

/** Distinct palette for routes so they read differently from FCAs on the map. */
const ROUTE_COLORS = [
  "#38bdf8",
  "#22d3ee",
  "#34d399",
  "#a78bfa",
  "#f472b6",
  "#facc15",
];

/** Editing state for a route (a filed-route string, not a drawn line). */
type RouteForm = {
  id: string | null;
  name: string;
  route: string;
  dep: string;
  arr: string;
  color: string;
};
function blankRouteForm(count: number): RouteForm {
  return {
    id: null,
    name: `Route ${count + 1}`,
    route: "",
    dep: "",
    arr: "",
    color: ROUTE_COLORS[count % ROUTE_COLORS.length],
  };
}
function routeFormFrom(r: MapRoute): RouteForm {
  return {
    id: r.id,
    name: r.name,
    route: r.route,
    dep: r.dep,
    arr: r.arr,
    color: r.color,
  };
}

function aircraftIcon(heading: number) {
  return L.divIcon({
    className: "",
    html: `<svg width="12" height="12" viewBox="0 0 12 12" style="transform: rotate(${heading}deg)"><path d="M6 0 L10.5 11 L6 8.5 L1.5 11 Z" fill="#22d3ee" fill-opacity="0.85"/></svg>`,
    iconSize: [12, 12],
    iconAnchor: [6, 6],
  });
}
function vertexIcon(color: string) {
  return L.divIcon({
    className: "",
    html: `<div style="width:12px;height:12px;border-radius:50%;background:#fff;border:2px solid ${color};box-shadow:0 0 0 1px rgba(0,0,0,.4)"></div>`,
    iconSize: [12, 12],
    iconAnchor: [6, 6],
  });
}
function labelIcon(color: string, name: string) {
  // Centered on the anchor and lifted above the line (no connector to it).
  return L.divIcon({
    className: "",
    html: `<div style="display:flex;align-items:center;gap:5px;white-space:nowrap;font:600 12px ui-monospace,monospace;color:${color};text-shadow:0 1px 3px #000,0 0 4px #000;transform:translate(-50%,calc(-100% - 9px))">
      <span style="display:block;width:4px;height:13px;background:${color};box-shadow:0 0 0 1px rgba(0,0,0,.4)"></span>${name}
    </div>`,
    iconSize: [0, 0],
    iconAnchor: [0, 0],
  });
}
/** A matched (crossing) aircraft: the plane glyph tinted the FCA colour, with its
 *  crossing-sequence number as a small badge beside it. */
function matchedIcon(seq: number, color: string, heading: number) {
  return L.divIcon({
    className: "",
    html: `<div style="position:relative;width:14px;height:14px">
      <svg width="14" height="14" viewBox="0 0 12 12" style="transform: rotate(${heading}deg)"><path d="M6 0 L10.5 11 L6 8.5 L1.5 11 Z" fill="${color}"/></svg>
      <span style="position:absolute;left:13px;top:-7px;height:14px;min-width:14px;padding:0 2px;border-radius:8px;background:${color};color:#0a0a0a;font:700 10px ui-monospace,monospace;line-height:14px;text-align:center;box-shadow:0 0 0 1px rgba(0,0,0,.4)">${seq}</span>
    </div>`,
    iconSize: [14, 14],
    iconAnchor: [7, 7],
  });
}
/** A small dot + name label for a route waypoint along a plotted route. */
function waypointLabelIcon(name: string) {
  return L.divIcon({
    className: "",
    html: `<div style="display:flex;align-items:center;gap:3px;transform:translate(4px,-1px)">
      <span style="display:block;width:4px;height:4px;border-radius:9999px;background:#38bdf8;box-shadow:0 0 0 1px rgba(0,0,0,.6)"></span>
      <span style="font:600 10px ui-monospace,monospace;color:#7dd3fc;white-space:nowrap;text-shadow:0 0 3px #000,0 1px 2px #000">${name}</span>
    </div>`,
    iconSize: [0, 0],
    iconAnchor: [0, 0],
  });
}
/** A route fix's name, offset from its dot (the dot is a separate centered circleMarker). */
function fixNameIcon(name: string, color: string) {
  return L.divIcon({
    className: "",
    html: `<span style="display:inline-block;font:600 10px ui-monospace,monospace;color:${color};white-space:nowrap;text-shadow:0 0 3px #000,0 1px 2px #000;transform:translate(6px,-2px)">${name}</span>`,
    iconSize: [0, 0],
    iconAnchor: [0, 0],
  });
}
function aircraftTip(ac: {
  callsign: string;
  actype: string;
  dep: string;
  arr: string;
  alt: number;
  gs: number;
}): string {
  return `<div style="font:700 13px ui-monospace,monospace"><span style="color:#22d3ee">${ac.callsign}</span> <span style="color:#94a3b8">${ac.actype}</span></div>
    <div style="font:12px ui-monospace,monospace;color:#cbd5e1">${ac.dep} → ${ac.arr}</div>
    <div style="font:12px ui-monospace,monospace;color:#94a3b8">FL${Math.round(ac.alt / 100)} ${ac.gs}kt</div>`;
}

export function FcaPage() {
  const { data: me } = useMe();
  const canRead = hasPermission(me, "flow.fca.read");
  const canEdit = hasPermission(me, "flow.fca.update");
  const canDelete = hasPermission(me, "flow.fca.delete");

  const fcas = useFcas();
  const traffic = useTraffic();
  const createFca = useCreateFca();
  const updateFca = useUpdateFca();
  const deleteFca = useDeleteFca();

  // Shared named routes (polylines drawn on the same map).
  const routes = useRoutes();
  const createRoute = useCreateRoute();
  const updateRoute = useUpdateRoute();
  const deleteRoute = useDeleteRoute();
  const canEditRoute = hasPermission(me, "flow.route.update");
  const canDeleteRoute = hasPermission(me, "flow.route.delete");

  const [draft, setDraft] = useState<Draft | null>(null);
  const [phase, setPhase] = useState<Phase>("draw");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [selectedRouteId, setSelectedRouteId] = useState<string | null>(null);
  const [routeForm, setRouteForm] = useState<RouteForm | null>(null);
  // Route ids whose fix (waypoint) names are shown on the map — toggled per route.
  const [labeledRoutes, setLabeledRoutes] = useState<Set<string>>(new Set());
  const [routeCallsign, setRouteCallsign] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [artccFilter, setArtccFilter] = useState("");
  // Which world copies are visible, as a stable key ("-360,0,360"). Bumped on pan/zoom so the
  // overlay layers re-draw themselves onto every visible copy of the world.
  const [offsetsKey, setOffsetsKey] = useState("0");

  const fcaTraffic = useFcaTraffic(draft ? null : selectedId);
  const counts = useFcaCounts();
  const aircraftRoute = useAircraftRoute(routeCallsign);
  const dataStatus = useDataStatus();
  const refreshData = useRefreshData();
  const cycleAge = dataStatus.data ? cycleAgeDays(dataStatus.data.nav_cycle) : null;
  const navStale = cycleAge != null && cycleAge > 35;

  const { resolvedTheme } = useTheme();

  // --- Leaflet refs ---
  const mapRef = useRef<L.Map | null>(null);
  const tileRef = useRef<L.TileLayer | null>(null);
  const roRef = useRef<ResizeObserver | null>(null);
  // Flips true once the map + layer groups exist, so the drawing effects below re-run and
  // paint even when their data resolved before the map mounted.
  const [mapReady, setMapReady] = useState(false);
  const boundaryLayer = useRef<L.LayerGroup | null>(null);
  const aircraftLayer = useRef<L.LayerGroup | null>(null);
  const fcaLayer = useRef<L.LayerGroup | null>(null);
  const matchedLayer = useRef<L.LayerGroup | null>(null);
  const routeLayer = useRef<L.LayerGroup | null>(null);
  const namedRouteLayer = useRef<L.LayerGroup | null>(null);
  const draftLayer = useRef<L.LayerGroup | null>(null);
  const drawingRef = useRef(false);
  useEffect(() => {
    drawingRef.current = !!draft && phase === "draw";
  }, [draft, phase]);

  const finishLine = () =>
    setDraft((d) => {
      if (d && d.points.length >= 2) setPhase("edit");
      return d;
    });

  // Initialize (and tear down) the map via a callback ref rather than a one-shot effect,
  // so it's created whenever the container actually mounts — including a hard refresh where
  // the container appears only after `me`/permissions resolve (a `[]` effect would have
  // already run as a no-op and never retried, leaving the map black).
  const setContainer = useCallback((node: HTMLDivElement | null) => {
    if (!node) {
      roRef.current?.disconnect();
      roRef.current = null;
      mapRef.current?.remove();
      mapRef.current = null;
      tileRef.current = null;
      setMapReady(false);
      return;
    }
    if (mapRef.current) return;
    const map = L.map(node, {
      zoomControl: false,
      doubleClickZoom: false,
    }).setView([38.5, -77], 6);
    L.control.zoom({ position: "topright" }).addTo(map);
    node.style.background = MAP_BG[resolvedTheme];
    tileRef.current = L.tileLayer(CARTO[resolvedTheme], {
      maxZoom: 14,
      attribution:
        "© OpenStreetMap, © CARTO · traffic: VATSIM · boundaries: FAA NASR / ERAM",
    }).addTo(map);

    // ARTCC boundaries render below everything, redrawn per visible world copy in an effect.
    boundaryLayer.current = L.layerGroup().addTo(map);
    routeLayer.current = L.layerGroup().addTo(map);
    namedRouteLayer.current = L.layerGroup().addTo(map);
    fcaLayer.current = L.layerGroup().addTo(map);
    aircraftLayer.current = L.layerGroup().addTo(map);
    matchedLayer.current = L.layerGroup().addTo(map);
    draftLayer.current = L.layerGroup().addTo(map);

    map.on("click", (e: L.LeafletMouseEvent) => {
      if (!drawingRef.current) return;
      // Keep the clicked longitude as-is (even on a wrapped copy) so the line appears where
      // it's drawn; longitudes are normalized to ±180 on save.
      const p: LatLng = [e.latlng.lat, e.latlng.lng];
      setDraft((d) => {
        if (!d) return d;
        const last = d.points[d.points.length - 1];
        if (last && haversine(last, p) < 0.4) return d; // dedupe dbl-click
        return { ...d, points: [...d.points, p] };
      });
    });
    map.on("dblclick", () => {
      if (drawingRef.current) finishLine();
    });

    // Track which world copies are on screen; overlay effects redraw when this set changes.
    const syncOffsets = () => {
      const key = copyOffsets(map).join(",");
      setOffsetsKey((cur) => (cur === key ? cur : key));
    };
    map.on("moveend zoomend", syncOffsets);
    syncOffsets();

    mapRef.current = map;
    // The container can still be sizing when the map inits (full-bleed layout settles late),
    // which would leave Leaflet with no tiles. Recompute size on every resize so tiles load
    // as soon as it has real dimensions.
    const ro = new ResizeObserver(() => map.invalidateSize());
    ro.observe(node);
    roRef.current = ro;
    map.invalidateSize();
    setMapReady(true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // React to light/dark toggles: swap the basemap tiles and map background.
  useEffect(() => {
    tileRef.current?.setUrl(CARTO[resolvedTheme]);
    const c = mapRef.current?.getContainer();
    if (c) c.style.background = MAP_BG[resolvedTheme];
  }, [resolvedTheme]);

  // Keyboard while drawing: Enter finish · Esc cancel · Backspace undo.
  useEffect(() => {
    if (!draft || phase !== "draw") return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        finishLine();
      } else if (e.key === "Escape") {
        setDraft(null);
      } else if (e.key === "Backspace") {
        e.preventDefault();
        setDraft((d) => (d ? { ...d, points: d.points.slice(0, -1) } : d));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [draft, phase]);

  // ARTCC boundary outlines + labels, drawn on every visible copy of the world.
  useEffect(() => {
    const layer = boundaryLayer.current;
    if (!layer) return;
    layer.clearLayers();
    const offsets = offsetsKey.split(",").map(Number);
    const fc = boundariesGeo as GeoJSON.FeatureCollection;
    for (const off of offsets) {
      for (const feat of fc.features) {
        const geom = feat.geometry;
        const rings: number[][][] =
          geom.type === "Polygon"
            ? geom.coordinates
            : geom.type === "MultiPolygon"
              ? geom.coordinates.flat()
              : [];
        if (rings.length === 0) continue;
        for (const ring of rings) {
          L.polyline(
            ring.map(([lon, lat]) => [lat, lon + off] as LatLng),
            { color: "#64748b", weight: 1, opacity: 0.4, interactive: false },
          ).addTo(layer);
        }
        const outer = rings[0];
        let sx = 0;
        let sy = 0;
        for (const [lon, lat] of outer) {
          sx += lon;
          sy += lat;
        }
        L.marker([sy / outer.length, sx / outer.length + off], {
          icon: L.divIcon({
            className: "",
            html: `<span style="color:#64748b;font:600 11px ui-monospace,monospace;opacity:.5">${feat.properties?.id ?? ""}</span>`,
            iconSize: [0, 0],
          }),
          interactive: false,
          keyboard: false,
        }).addTo(layer);
      }
    }
  }, [offsetsKey, mapReady]);

  // Live aircraft — hover for details, click to plot the route. Aircraft matched to
  // the selected FCA are drawn (tinted + numbered) by the matched layer instead. Drawn on
  // every visible world copy so they persist as you scroll.
  useEffect(() => {
    const layer = aircraftLayer.current;
    if (!layer) return;
    layer.clearLayers();
    const offsets = offsetsKey.split(",").map(Number);
    const matched = new Set((fcaTraffic.data ?? []).map((f) => f.callsign));
    for (const ac of traffic.data ?? []) {
      if (matched.has(ac.callsign)) continue;
      for (const off of offsets) {
        L.marker([ac.lat, ac.lon + off], {
          icon: aircraftIcon(ac.heading),
          keyboard: false,
        })
          .bindTooltip(aircraftTip(ac), {
            direction: "top",
            offset: [0, -6],
            className: "fca-tip",
          })
          .on("click", (e) => {
            L.DomEvent.stop(e);
            setRouteCallsign((cur) => (cur === ac.callsign ? null : ac.callsign));
          })
          .addTo(layer);
      }
    }
  }, [traffic.data, fcaTraffic.data, offsetsKey, mapReady]);

  // Plotted route for a clicked aircraft.
  useEffect(() => {
    const layer = routeLayer.current;
    if (!layer) return;
    layer.clearLayers();
    const raw = aircraftRoute.data?.points as LatLng[] | undefined;
    if (raw && raw.length >= 2) {
      const base = unwrapLng(raw);
      for (const off of offsetsKey.split(",").map(Number)) {
        const pts = shiftLine(base, off);
        L.polyline(pts, {
          color: "#22d3ee",
          weight: 2,
          opacity: 0.85,
          dashArray: "6 6",
          interactive: false,
        }).addTo(layer);
        for (const end of [pts[0], pts[pts.length - 1]]) {
          L.circleMarker(end, {
            radius: 4,
            color: "#22d3ee",
            weight: 1,
            fillColor: "#22d3ee",
            fillOpacity: 1,
            interactive: false,
          }).addTo(layer);
        }
        for (const wp of aircraftRoute.data?.waypoints ?? []) {
          L.marker([wp.lat, wp.lon + off], {
            icon: waypointLabelIcon(wp.name),
            interactive: false,
            keyboard: false,
            zIndexOffset: -200,
          }).addTo(layer);
        }
      }
    }
  }, [aircraftRoute.data, offsetsKey, mapReady]);

  // Saved FCAs (skip the one being edited — drawn on the draft layer).
  useEffect(() => {
    const layer = fcaLayer.current;
    if (!layer) return;
    layer.clearLayers();
    for (const fca of fcas.data ?? []) {
      if (fca.id === draft?.id) continue;
      const raw = fca.points as LatLng[];
      if (!raw || raw.length < 2) continue;
      const base = unwrapLng(raw);
      const selected = fca.id === selectedId;
      const opacity = fca.enabled ? (selected ? 1 : 0.85) : 0.3;
      for (const off of offsetsKey.split(",").map(Number)) {
        const pts = shiftLine(base, off);
        L.polyline(pts, {
          color: fca.color,
          weight: selected ? 4 : 3,
          opacity,
          dashArray: "4 8",
        })
          .on("click", (e) => {
            L.DomEvent.stop(e);
            setSelectedId((cur) => (cur === fca.id ? null : fca.id));
          })
          .addTo(layer);
        for (const end of [pts[0], pts[pts.length - 1]]) {
          L.circleMarker(end, {
            radius: selected ? 5 : 4,
            color: fca.color,
            weight: 1,
            fillColor: fca.color,
            fillOpacity: opacity,
            interactive: false,
          }).addTo(layer);
        }
        L.marker(midpointOf(pts), {
          icon: labelIcon(fca.color, fca.name),
          interactive: false,
          keyboard: false,
        }).addTo(layer);
      }
    }
  }, [fcas.data, draft?.id, selectedId, offsetsKey, mapReady]);

  // Saved named routes — solid polylines (distinct from the dashed FCAs).
  useEffect(() => {
    const layer = namedRouteLayer.current;
    if (!layer) return;
    layer.clearLayers();
    for (const r of routes.data ?? []) {
      if (r.id === draft?.id) continue; // the one being edited is on the draft layer
      const raw = r.points as LatLng[];
      if (!raw || raw.length < 2) continue;
      const base = unwrapLng(raw);
      const selected = r.id === selectedRouteId;
      const showFixes = labeledRoutes.has(r.id);
      for (const off of offsetsKey.split(",").map(Number)) {
        const pts = shiftLine(base, off);
        // Routes are display-only — not clickable on the map.
        L.polyline(pts, {
          color: r.color,
          weight: selected ? 5 : 3,
          opacity: selected ? 1 : 0.85,
          interactive: false,
        }).addTo(layer);
        for (const end of [pts[0], pts[pts.length - 1]]) {
          L.circleMarker(end, {
            radius: selected ? 5 : 4,
            color: r.color,
            weight: 1,
            fillColor: r.color,
            fillOpacity: 0.9,
            interactive: false,
          }).addTo(layer);
        }
        L.marker(midpointOf(pts), {
          icon: labelIcon(r.color, r.name),
          interactive: false,
          keyboard: false,
        }).addTo(layer);
        // Per-route fixes (toggled): a tiny dot at each fix + its name beside it.
        if (showFixes) {
          for (const wp of r.waypoints) {
            const at: LatLng = [wp.lat, wp.lon + off];
            L.circleMarker(at, {
              radius: 2.5,
              color: r.color,
              weight: 1,
              fillColor: r.color,
              fillOpacity: 1,
              interactive: false,
            }).addTo(layer);
            L.marker(at, {
              icon: fixNameIcon(wp.name, r.color),
              interactive: false,
              keyboard: false,
              zIndexOffset: -100,
            }).addTo(layer);
          }
        }
      }
    }
  }, [routes.data, draft?.id, selectedRouteId, labeledRoutes, offsetsKey, mapReady]);

  // Matched (crossing) traffic for the selected FCA — numbered, in the FCA colour.
  const selectedColor = fcas.data?.find((f) => f.id === selectedId)?.color;
  useEffect(() => {
    const layer = matchedLayer.current;
    if (!layer) return;
    layer.clearLayers();
    if (draft || !selectedColor) return;
    const color = selectedColor;
    const offsets = offsetsKey.split(",").map(Number);
    for (const f of fcaTraffic.data ?? []) {
      const hasPos = f.lat !== 0 || f.lon !== 0;
      const path = f.path as LatLng[] | undefined;
      const line =
        path && path.length >= 2
          ? path
          : ([
              [f.lat, f.lon],
              [f.cross_lat, f.cross_lon],
            ] as LatLng[]);
      const base = unwrapLng(line);
      for (const off of offsets) {
        if (hasPos) {
          L.polyline(shiftLine(base, off), {
            color,
            weight: 1.5,
            opacity: 0.55,
            interactive: false,
          }).addTo(layer);
        }
        L.circleMarker([f.cross_lat, f.cross_lon + off], {
          radius: 3,
          color: "#ffffff",
          weight: 1,
          fillColor: "#ffffff",
          fillOpacity: 0.9,
          interactive: false,
        }).addTo(layer);
        if (hasPos) {
          L.marker([f.lat, f.lon + off], {
            icon: matchedIcon(f.seq, color, f.heading),
            keyboard: false,
          })
            .bindTooltip(
              `<div style="font:700 13px ui-monospace,monospace">#${f.seq} <span style="color:${color}">${f.callsign}</span> <span style="color:#94a3b8">${f.aircraft_type}</span></div>
             <div style="font:12px ui-monospace,monospace;color:#cbd5e1">${f.dep} → ${f.arr}</div>
             <div style="font:12px ui-monospace,monospace;color:#94a3b8">FL${Math.round(f.altitude / 100)} ${f.groundspeed}kt · ${Math.round(f.distance_nm)}nm to line</div>`,
              { direction: "top", offset: [0, -8], className: "fca-tip" },
            )
            .on("click", (e) => {
              L.DomEvent.stop(e);
              setRouteCallsign((cur) => (cur === f.callsign ? null : f.callsign));
            })
            .addTo(layer);
        }
      }
    }
  }, [fcaTraffic.data, draft, selectedColor, offsetsKey, mapReady]);

  // Working draft (dashed polyline + draggable vertex handles).
  useEffect(() => {
    const layer = draftLayer.current;
    if (!layer) return;
    layer.clearLayers();
    if (!draft) return;
    if (draft.points.length >= 2) {
      L.polyline(unwrapLng(draft.points), {
        color: draft.color,
        weight: 4,
        dashArray: "6 6",
      }).addTo(layer);
    }
    draft.points.forEach((pt, i) => {
      const handle = L.marker(pt, {
        draggable: true,
        icon: vertexIcon(draft.color),
      });
      handle.on("dragend", (e) => {
        const ll = (e.target as L.Marker).getLatLng();
        setDraft((prev) =>
          prev
            ? {
                ...prev,
                points: prev.points.map((p, idx) =>
                  idx === i ? [ll.lat, ll.lng] : p,
                ),
              }
            : prev,
        );
      });
      handle.addTo(layer);
    });
  }, [draft, mapReady]);

  // Zoom to the selected FCA.
  useEffect(() => {
    const map = mapRef.current;
    if (!map || !selectedId) return;
    const fca = fcas.data?.find((f) => f.id === selectedId);
    const pts = (fca?.points as LatLng[]) ?? [];
    if (pts.length >= 2) {
      map.fitBounds(pts as L.LatLngBoundsExpression, {
        padding: [80, 80],
        maxZoom: 9,
      });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedId]);

  const startNew = () => {
    setSelectedId(null);
    setRouteCallsign(null);
    setDraft(blankDraft(fcas.data?.length ?? 0));
    setPhase("draw");
  };
  const startEdit = (fca: Fca) => {
    setSelectedId(null);
    setRouteCallsign(null);
    setDraft(draftFrom(fca));
    setPhase("edit");
  };
  // Routes are edited as a filed-route string (not drawn on the map).
  const startNewRoute = () => {
    setSelectedRouteId(null);
    setRouteForm(blankRouteForm(routes.data?.length ?? 0));
  };
  const startEditRoute = (r: MapRoute) => {
    setSelectedRouteId(null);
    setRouteForm(routeFormFrom(r));
  };
  const toggleFixes = (id: string) =>
    setLabeledRoutes((s) => {
      const next = new Set(s);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  const saveRoute = () => {
    if (!routeForm || !routeForm.name.trim() || !routeForm.route.trim()) return;
    const body: UpsertRoute = {
      name: routeForm.name.trim(),
      color: routeForm.color,
      route: routeForm.route.trim().toUpperCase(),
      dep: routeForm.dep.trim().toUpperCase(),
      arr: routeForm.arr.trim().toUpperCase(),
    };
    const done = () => setRouteForm(null);
    if (routeForm.id) {
      updateRoute.mutate({ id: routeForm.id, body }, { onSuccess: done });
    } else {
      createRoute.mutate(body, { onSuccess: done });
    }
  };
  const cancel = () => setDraft(null);

  const save = () => {
    if (!draft || draft.points.length < 2) return;
    const body: UpsertFca = {
      name: draft.name.trim() || "FCA",
      color: draft.color,
      artcc: draft.artcc.trim().toUpperCase(),
      points: normPoints(draft.points),
      dests: parseList(draft.dests),
      origins: parseList(draft.origins),
      fixes: parseList(draft.fixes),
      scope: parseList(draft.scope),
      min_fl: parseFl(draft.minFl),
      max_fl: parseFl(draft.maxFl),
      mode: draft.mode,
      rate: draft.rate,
      mit: draft.mit,
    };
    if (draft.id) {
      updateFca.mutate({ id: draft.id, body }, { onSuccess: cancel });
    } else {
      createFca.mutate(body, { onSuccess: cancel });
    }
  };

  const toggleEnabled = (fca: Fca) => {
    updateFca.mutate({ id: fca.id, body: { ...toUpsert(fca), enabled: !fca.enabled } });
  };

  const shown = useMemo(() => {
    const q = filter.trim().toUpperCase();
    return (fcas.data ?? []).filter(
      (f) =>
        (!q ||
          f.name.toUpperCase().includes(q) ||
          f.artcc.toUpperCase().includes(q)) &&
        (!artccFilter || f.artcc === artccFilter),
    );
  }, [fcas.data, filter, artccFilter]);

  const artccOptions = useMemo(() => {
    const s = new Set((fcas.data ?? []).map((f) => f.artcc).filter(Boolean));
    return [...s].sort();
  }, [fcas.data]);

  const selectedFca = fcas.data?.find((f) => f.id === selectedId);
  const editing = !!draft && phase === "edit";
  const drawing = !!draft && phase === "draw";

  if (!canRead) {
    return (
      <div className="flex h-[calc(100vh-3.5rem)] items-center justify-center text-sm text-muted-foreground">
        You don&apos;t have flow access.
      </div>
    );
  }

  return (
    <div className="flex h-[calc(100vh-3.5rem)]">
      {/* Sidebar */}
      <aside className="flex w-80 shrink-0 flex-col border-r bg-background">
        <div className="flex items-center justify-between border-b px-4 py-3">
          <span className="text-sm font-semibold uppercase tracking-wide">
            Flow Constrained Areas
          </span>
        </div>

        {editing ? (
          <DraftEditor
            draft={draft}
            onChange={setDraft}
            onSave={save}
            onRedraw={() => {
              setDraft((d) => (d ? { ...d, points: [] } : d));
              setPhase("draw");
            }}
            onCancel={cancel}
            saving={createFca.isPending || updateFca.isPending}
          />
        ) : (
          <>
            {canEdit && (
              <div className="border-b p-3">
                {drawing ? (
                  <Button
                    variant="secondary"
                    className="w-full"
                    onClick={cancel}
                  >
                    Cancel drawing
                  </Button>
                ) : (
                  <Button className="w-full" onClick={startNew}>
                    <Plus />
                    New FCA
                  </Button>
                )}
              </div>
            )}
            <div className="flex flex-col gap-2 border-b p-3">
              <select
                value={artccFilter}
                onChange={(e) => setArtccFilter(e.target.value)}
                className="h-9 w-full rounded-md border border-input bg-background px-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <option value="">ALL ARTCCs</option>
                {artccOptions.map((a) => (
                  <option key={a} value={a}>
                    {a}
                  </option>
                ))}
              </select>
              <Input
                placeholder="filter — name or fix…"
                value={filter}
                onChange={(e) => setFilter(e.target.value)}
              />
            </div>
            <div className="flex-1 overflow-y-auto">
              {!fcas.data ? (
                <p className="p-4 text-sm text-muted-foreground">Loading…</p>
              ) : shown.length === 0 ? (
                <p className="p-4 text-sm text-muted-foreground">
                  No FCAs yet. {canEdit && "Draw one with “New FCA”."}
                </p>
              ) : (
                <ul>
                  {shown.map((fca) => (
                    <li
                      key={fca.id}
                      className={
                        "flex items-center gap-2 border-b px-3 py-2 text-sm " +
                        (fca.id === selectedId ? "bg-accent/40" : "")
                      }
                    >
                      <button
                        type="button"
                        title={fca.enabled ? "Enabled" : "Disabled"}
                        onClick={() => canEdit && toggleEnabled(fca)}
                        className="size-3 shrink-0 rounded-full"
                        style={{
                          background: fca.enabled ? fca.color : "transparent",
                          border: `2px solid ${fca.color}`,
                        }}
                      />
                      <button
                        type="button"
                        onClick={() =>
                          setSelectedId((cur) => (cur === fca.id ? null : fca.id))
                        }
                        className="flex-1 truncate text-left font-mono"
                      >
                        {fca.name}
                        {fca.artcc && (
                          <span className="ml-1.5 text-xs text-muted-foreground">
                            {fca.artcc}
                          </span>
                        )}
                      </button>
                      {(() => {
                        const c = counts.data?.[fca.id] ?? 0;
                        return (
                          <span
                            className={
                              "shrink-0 rounded px-1.5 text-xs font-medium tabular-nums " +
                              (c > 0
                                ? "bg-primary/15 text-primary"
                                : "text-muted-foreground/50")
                            }
                          >
                            {c}
                          </span>
                        );
                      })()}
                      {canEdit && (
                        <button
                          type="button"
                          title="Edit"
                          onClick={() => startEdit(fca)}
                          className="text-muted-foreground hover:text-foreground"
                        >
                          <Pencil className="size-3.5" />
                        </button>
                      )}
                      {canDelete && (
                        <ConfirmButton
                          size="icon"
                          className="size-7"
                          title="Delete"
                          aria-label="Delete FCA"
                          onConfirm={() => deleteFca.mutate(fca.id)}
                          warn={`Delete the “${fca.name}” FCA?`}
                        >
                          <Trash2 className="size-3.5" />
                        </ConfirmButton>
                      )}
                    </li>
                  ))}
                </ul>
              )}
            </div>

            {/* Shared routes (filed-route strings resolved by the nav engine) */}
            <div className="flex flex-col border-t">
              {routeForm ? (
                <RouteEditor
                  form={routeForm}
                  onChange={setRouteForm}
                  onSave={saveRoute}
                  onCancel={() => setRouteForm(null)}
                  saving={createRoute.isPending || updateRoute.isPending}
                />
              ) : (
                <>
                  <div className="flex items-center justify-between px-3 pt-3">
                    <span className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                      Routes
                    </span>
                    {canEditRoute && (
                      <button
                        type="button"
                        onClick={startNewRoute}
                        className="flex items-center gap-1 text-xs text-primary hover:underline"
                      >
                        <Plus className="size-3.5" />
                        New route
                      </button>
                    )}
                  </div>
                  {routes.data && routes.data.length > 0 ? (
                    <ul className="max-h-48 overflow-y-auto p-1">
                      {routes.data.map((r) => (
                        <li
                          key={r.id}
                          className={
                            "flex items-center gap-2 rounded px-2 py-1.5 text-sm " +
                            (r.id === selectedRouteId ? "bg-accent/40" : "")
                          }
                        >
                          <span
                            className="size-3 shrink-0 rounded-full"
                            style={{ background: r.color, border: `2px solid ${r.color}` }}
                          />
                          <button
                            type="button"
                            title={r.route}
                            onClick={() =>
                              setSelectedRouteId((cur) => (cur === r.id ? null : r.id))
                            }
                            className="flex-1 truncate text-left font-mono"
                          >
                            {r.name}
                            {r.unresolved.length > 0 && (
                              <span className="ml-1.5 text-xs text-amber-500" title={`Unresolved: ${r.unresolved.join(" ")}`}>
                                ⚠{r.unresolved.length}
                              </span>
                            )}
                          </button>
                          <button
                            type="button"
                            title={
                              labeledRoutes.has(r.id)
                                ? "Hide fix names"
                                : "Show fix names"
                            }
                            onClick={() => toggleFixes(r.id)}
                            className={
                              "transition-colors " +
                              (labeledRoutes.has(r.id)
                                ? "text-primary"
                                : "text-muted-foreground hover:text-foreground")
                            }
                          >
                            <Tag className="size-3.5" />
                          </button>
                          {canEditRoute && (
                            <button
                              type="button"
                              title="Edit"
                              onClick={() => startEditRoute(r)}
                              className="text-muted-foreground hover:text-foreground"
                            >
                              <Pencil className="size-3.5" />
                            </button>
                          )}
                          {canDeleteRoute && (
                            <ConfirmButton
                              size="icon"
                              className="size-7"
                              title="Delete"
                              aria-label="Delete route"
                              onConfirm={() => deleteRoute.mutate(r.id)}
                              warn={`Delete the “${r.name}” route?`}
                            >
                              <Trash2 className="size-3.5" />
                            </ConfirmButton>
                          )}
                        </li>
                      ))}
                    </ul>
                  ) : (
                    <p className="px-3 py-3 text-xs text-muted-foreground">
                      No routes yet.
                      {canEditRoute && " Add one with “New route”."}
                    </p>
                  )}
                </>
              )}
            </div>
          </>
        )}

        <div className="border-t px-4 py-2 text-xs">
          {dataStatus.data ? (
            <div className="flex items-center justify-between gap-2">
              <div className="min-w-0">
                <div className="flex items-center gap-1.5 font-mono">
                  <span
                    className={`size-1.5 shrink-0 rounded-full ${
                      navStale ? "bg-amber-500" : "bg-emerald-500"
                    }`}
                  />
                  <span>NASR {dataStatus.data.nav_cycle}</span>
                  {cycleAge != null && (
                    <span
                      className={
                        navStale ? "text-amber-500" : "text-muted-foreground"
                      }
                    >
                      · {cycleAge}d
                    </span>
                  )}
                </div>
                <div className="truncate text-muted-foreground">
                  {dataStatus.data.winds_stations} winds ·{" "}
                  {traffic.data?.length ?? 0} traffic
                </div>
              </div>
              {canEdit && (
                <button
                  type="button"
                  onClick={() => refreshData.mutate()}
                  disabled={refreshData.isPending}
                  title="Refresh nav + winds now"
                  className="shrink-0 rounded p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:opacity-50"
                >
                  <RefreshCw
                    className={`size-3.5 ${refreshData.isPending ? "animate-spin" : ""}`}
                  />
                </button>
              )}
            </div>
          ) : (
            <span className="text-muted-foreground">
              {traffic.data ? `traffic ${traffic.data.length}` : "traffic…"}
            </span>
          )}
        </div>
        <CoveragePanel />
      </aside>

      {/* Map — `isolate` traps Leaflet z-indexes below the navbar dropdowns. */}
      <div className="relative isolate flex-1">
        <div ref={setContainer} className="absolute inset-0" />

        {navStale && (
          <div className="pointer-events-none absolute inset-x-0 top-3 z-[500] flex justify-center">
            <div className="pointer-events-auto flex items-center gap-2 rounded-lg border border-amber-500/50 bg-amber-500/10 px-3 py-1.5 text-xs text-amber-700 shadow-lg backdrop-blur dark:text-amber-200">
              <RefreshCw className="size-3.5" />
              <span>
                NASR data is {cycleAge} days old ({dataStatus.data?.nav_cycle}).
              </span>
              {canEdit && (
                <button
                  type="button"
                  onClick={() => refreshData.mutate()}
                  disabled={refreshData.isPending}
                  className="font-semibold underline underline-offset-2 disabled:opacity-50"
                >
                  Refresh now
                </button>
              )}
            </div>
          </div>
        )}
        {drawing && (
          <div className="pointer-events-none absolute inset-x-0 bottom-6 z-[500] flex justify-center">
            <div className="pointer-events-auto flex items-center gap-2.5 rounded-lg border border-primary/60 bg-background/95 px-5 py-3 text-sm shadow-lg backdrop-blur">
              <span className="font-medium">Click to add points</span>
              <Kbd>⌫</Kbd>
              <span className="text-muted-foreground">undo</span>
              <Kbd>dbl-click</Kbd>
              <span className="text-muted-foreground">or</span>
              <Kbd>↵</Kbd>
              <span className="text-muted-foreground">finish</span>
              <Kbd>esc</Kbd>
              <span className="text-muted-foreground">cancel</span>
              <span className="ml-1 border-l pl-3 font-mono text-xs text-muted-foreground">
                {draft.points.length} pts · {Math.round(lineNm(draft.points))} nm
              </span>
            </div>
          </div>
        )}

        {routeCallsign && aircraftRoute.data && (
          <RoutePopup
            route={aircraftRoute.data}
            fca={selectedFca}
            match={fcaTraffic.data?.find(
              (f) => f.callsign === aircraftRoute.data!.callsign,
            )}
            onClose={() => setRouteCallsign(null)}
          />
        )}
      </div>

      {/* Detail board for the selected FCA (metering ladder + strips). */}
      {selectedFca && !draft && (
        <FcaDetail
          fca={selectedFca}
          flights={fcaTraffic.data}
          canEdit={canEdit}
        />
      )}
    </div>
  );
}

/** Compact, expandable route-resolve coverage for the current live traffic. */
function CoveragePanel() {
  const coverage = useRouteCoverage();
  const [open, setOpen] = useState(false);
  const c = coverage.data;
  if (!c || c.pilots_with_route === 0) return null;
  const pct = c.resolved_pct;
  const tone =
    pct >= 90 ? "text-emerald-500" : pct >= 75 ? "text-amber-500" : "text-red-500";
  return (
    <div className="border-t px-4 py-2 text-xs">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        className="flex w-full items-center justify-between gap-2 text-left"
        title="Share of live filed routes the nav engine fully resolves"
      >
        <span className="text-muted-foreground">Route coverage</span>
        <span className="font-mono tabular-nums">
          <span className={tone}>{pct.toFixed(0)}%</span>
          <span className="text-muted-foreground">
            {" "}
            ({c.fully_resolved}/{c.pilots_with_route})
          </span>
          <ChevronDown
            className={`ml-1 inline size-3 transition-transform ${open ? "rotate-180" : ""}`}
          />
        </span>
      </button>
      {open && c.top_unresolved.length > 0 && (
        <div className="mt-1.5 max-h-40 overflow-y-auto rounded border bg-muted/20 p-1.5">
          <div className="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">
            Top unresolved tokens
          </div>
          <div className="grid grid-cols-2 gap-x-3 gap-y-0.5 font-mono">
            {c.top_unresolved.slice(0, 20).map((u) => (
              <div key={u.token} className="flex justify-between gap-2">
                <span className="truncate">{u.token}</span>
                <span className="shrink-0 text-muted-foreground">{u.count}</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function RoutePopup({
  route,
  fca,
  match,
  onClose,
}: {
  route: AircraftRoute;
  fca: Fca | null | undefined;
  match: FcaFlight | undefined;
  onClose: () => void;
}) {
  const pts = route.points as LatLng[];
  const nm = Math.round(lineNm(pts));
  const unresolved = route.unresolved ?? [];

  // Drag by the header; position is a pixel offset within the map, clamped so the card
  // can never leave the map region.
  const [pos, setPos] = useState({ x: 12, y: 12 });
  const [minimized, setMinimized] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const drag = useRef<{ px: number; py: number; ox: number; oy: number } | null>(
    null,
  );
  const clampPos = (x: number, y: number) => {
    const el = rootRef.current;
    const parent = el?.offsetParent as HTMLElement | null;
    if (!el || !parent) return { x, y };
    const maxX = Math.max(0, parent.clientWidth - el.offsetWidth);
    const maxY = Math.max(0, parent.clientHeight - el.offsetHeight);
    return {
      x: Math.min(Math.max(0, x), maxX),
      y: Math.min(Math.max(0, y), maxY),
    };
  };
  const onDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    drag.current = { px: e.clientX, py: e.clientY, ox: pos.x, oy: pos.y };
    e.currentTarget.setPointerCapture(e.pointerId);
  };
  const onMove = (e: React.PointerEvent) => {
    if (!drag.current) return;
    setPos(
      clampPos(
        drag.current.ox + (e.clientX - drag.current.px),
        drag.current.oy + (e.clientY - drag.current.py),
      ),
    );
  };
  const onUp = () => {
    drag.current = null;
  };
  const stopPointer = (e: React.PointerEvent) => e.stopPropagation();

  // Re-clamp when the card resizes (minimize/expand) or the map region resizes, so it
  // stays fully inside its new bounds.
  useEffect(() => {
    const reclamp = () => setPos((p) => clampPos(p.x, p.y));
    reclamp();
    window.addEventListener("resize", reclamp);
    return () => window.removeEventListener("resize", reclamp);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [minimized]);

  if (minimized) {
    return (
      <div
        ref={rootRef}
        className="absolute z-[600]"
        style={{ left: pos.x, top: pos.y }}
      >
        <div
          onPointerDown={onDown}
          onPointerMove={onMove}
          onPointerUp={onUp}
          className="flex cursor-move touch-none select-none items-center gap-2 rounded-lg border border-border/70 bg-background/95 px-2.5 py-1.5 shadow-2xl backdrop-blur"
        >
          <span className="font-mono text-sm font-bold text-sky-400">
            {route.callsign}
          </span>
          <button
            type="button"
            onPointerDown={stopPointer}
            onClick={() => setMinimized(false)}
            className="rounded p-0.5 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
            aria-label="Expand"
          >
            <Maximize2 className="size-3.5" />
          </button>
          <button
            type="button"
            onPointerDown={stopPointer}
            onClick={onClose}
            className="rounded p-0.5 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
            aria-label="Close"
          >
            <X className="size-3.5" />
          </button>
        </div>
      </div>
    );
  }

  return (
    <div
      ref={rootRef}
      className="absolute z-[600] w-[min(92vw,26rem)] rounded-xl border border-border/70 bg-background/95 p-4 shadow-2xl backdrop-blur"
      style={{ left: pos.x, top: pos.y }}
    >
      <div
        onPointerDown={onDown}
        onPointerMove={onMove}
        onPointerUp={onUp}
        className="flex cursor-move touch-none select-none items-start justify-between gap-3"
      >
        <div className="min-w-0 font-mono">
          <span className="text-lg font-bold tracking-tight text-sky-400">
            {route.callsign}
          </span>
          {route.aircraft_type && (
            <span className="ml-2 text-sm text-muted-foreground">
              {route.aircraft_type}
            </span>
          )}
        </div>
        <div className="-mr-1 -mt-1 flex items-center gap-0.5">
          <button
            type="button"
            onPointerDown={stopPointer}
            onClick={() => setMinimized(true)}
            className="rounded p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
            aria-label="Minimize"
          >
            <Minus className="size-4" />
          </button>
          <button
            type="button"
            onPointerDown={stopPointer}
            onClick={onClose}
            className="rounded p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
            aria-label="Close"
          >
            <X className="size-4" />
          </button>
        </div>
      </div>

      <div className="mt-1 font-mono text-sm text-muted-foreground">
        {route.dep || "????"} → {route.arr || "????"}
        {route.altitude > 0 && <> · FL{Math.round(route.altitude / 100)}</>}
        {route.groundspeed > 0 && <> · {route.groundspeed}kt</>}
        {nm > 0 && <> · {nm} NM</>} · {pts.length} pts
      </div>

      <div className="mt-3 max-h-40 overflow-y-auto rounded-lg border border-border/60 bg-muted/20 p-2.5 font-mono text-xs leading-relaxed break-words">
        {route.route || "(no filed route)"}
      </div>

      <p className="mt-2 text-[11px] leading-snug text-muted-foreground/70">
        FAA NASR route{route.nav_cycle ? ` (${route.nav_cycle})` : ""} — fixes,
        navaids, airways, SID/STAR when known.
        {unresolved.length > 0 && (
          <>
            {" "}
            <span className="text-amber-500/80">
              Unresolved: {unresolved.slice(0, 14).join(", ")}
              {unresolved.length > 14 ? "…" : ""}.
            </span>
          </>
        )}
      </p>

      {fca && (
        <div className="mt-3 flex items-baseline gap-1.5 border-t border-border/60 pt-2.5 font-mono text-xs">
          <span className="font-semibold" style={{ color: fca.color }}>
            {fca.name}
          </span>
          {match ? (
            <span className="text-emerald-400">
              IN SEQUENCE #{match.seq} — crosses in {match.distance_nm}nm.
            </span>
          ) : (
            <span className="text-muted-foreground">
              not crossing this FCA.
            </span>
          )}
        </div>
      )}
    </div>
  );
}

function Kbd({ children }: { children: React.ReactNode }) {
  return (
    <kbd className="rounded border bg-muted px-1.5 py-0.5 font-mono text-xs">
      {children}
    </kbd>
  );
}

function Field({
  label,
  help,
  children,
}: {
  label: string;
  help?: string;
  children: React.ReactNode;
}) {
  return (
    <label className="flex flex-col gap-1">
      <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
        {label}
      </span>
      {children}
      {help && <span className="text-[11px] leading-snug text-muted-foreground/80">{help}</span>}
    </label>
  );
}

function RouteEditor({
  form,
  onChange,
  onSave,
  onCancel,
  saving,
}: {
  form: RouteForm;
  onChange: (f: RouteForm) => void;
  onSave: () => void;
  onCancel: () => void;
  saving: boolean;
}) {
  const set = <K extends keyof RouteForm>(k: K, v: RouteForm[K]) =>
    onChange({ ...form, [k]: v });
  return (
    <div className="flex flex-col gap-3 p-3">
      <div className="text-sm font-semibold text-primary">
        {form.id ? "EDIT ROUTE" : "NEW ROUTE"}
      </div>
      <Field label="Name">
        <Input value={form.name} onChange={(e) => set("name", e.target.value)} />
      </Field>
      <Field
        label="Route"
        help="A filed-route string — fixes, navaids, airways, SID/STAR. The nav engine draws it."
      >
        <textarea
          value={form.route}
          onChange={(e) => set("route", e.target.value)}
          rows={3}
          placeholder="RBV Q430 BYRDD J48 MOL FLASK OZZZI2"
          className="w-full rounded-md border border-input bg-background px-3 py-2 font-mono text-sm uppercase outline-none focus-visible:ring-2 focus-visible:ring-ring"
        />
      </Field>
      <div className="grid grid-cols-2 gap-3">
        <Field label="Dep" help="Optional — improves SID / preferred-route resolution.">
          <Input
            className="font-mono uppercase"
            maxLength={4}
            placeholder="KJFK"
            value={form.dep}
            onChange={(e) => set("dep", e.target.value)}
          />
        </Field>
        <Field label="Arr" help="Optional — improves STAR resolution.">
          <Input
            className="font-mono uppercase"
            maxLength={4}
            placeholder="KBOS"
            value={form.arr}
            onChange={(e) => set("arr", e.target.value)}
          />
        </Field>
      </div>
      <div className="flex flex-col gap-1">
        <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
          Color
        </span>
        <div className="flex flex-wrap gap-1.5">
          {ROUTE_COLORS.map((c) => (
            <button
              key={c}
              type="button"
              onClick={() => set("color", c)}
              className={
                "size-6 rounded-full " +
                (form.color === c
                  ? "ring-2 ring-ring ring-offset-2 ring-offset-background"
                  : "")
              }
              style={{ background: c }}
            />
          ))}
        </div>
      </div>
      <div className="flex gap-2 pt-1">
        <Button
          className="flex-1"
          onClick={onSave}
          disabled={!form.name.trim() || !form.route.trim() || saving}
        >
          Save route
        </Button>
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </div>
  );
}

function DraftEditor({
  draft,
  onChange,
  onSave,
  onRedraw,
  onCancel,
  saving,
}: {
  draft: Draft;
  onChange: (d: Draft) => void;
  onSave: () => void;
  onRedraw: () => void;
  onCancel: () => void;
  saving: boolean;
}) {
  const set = <K extends keyof Draft>(k: K, v: Draft[K]) =>
    onChange({ ...draft, [k]: v });

  return (
    <div className="flex flex-1 flex-col gap-3 overflow-y-auto p-3">
      <div className="text-sm font-semibold text-primary">
        {draft.id ? "EDIT FCA" : "NEW FCA"} ·{" "}
        <span className="tabular-nums">{draft.points.length}</span> pts ·{" "}
        <span className="tabular-nums">{Math.round(lineNm(draft.points))}</span> nm
      </div>

      <Field label="Name">
        <Input value={draft.name} onChange={(e) => set("name", e.target.value)} />
      </Field>

      <Field
        label="Destination airports"
        help="Space/comma-separated ICAO. Blank meters every arrival crossing the line."
      >
        <Input
          className="font-mono uppercase"
          placeholder="KATL KCLT · blank = all"
          value={draft.dests}
          onChange={(e) => set("dests", e.target.value)}
        />
      </Field>

      <Field
        label="Departure airports"
        help="Only meter flights departing these fields (e.g. all KMCO departures 10 MIT). Blank = any departure."
      >
        <Input
          className="font-mono uppercase"
          placeholder="KMCO · blank = all"
          value={draft.origins}
          onChange={(e) => set("origins", e.target.value)}
        />
      </Field>

      <Field
        label="Route fixes"
        help="Only meter aircraft with these fixes in their FILED route (procedure names count: LAIRI matches a filed LAIRI4). Blank = any route."
      >
        <Input
          className="font-mono uppercase"
          placeholder="LAIRI · blank = all"
          value={draft.fixes}
          onChange={(e) => set("fixes", e.target.value)}
        />
      </Field>

      <div className="grid grid-cols-2 gap-3">
        <Field label="ARTCC tag" help="Owning facility — used by the list filter.">
          <Input
            className="font-mono uppercase"
            maxLength={4}
            placeholder="ZDC"
            value={draft.artcc}
            onChange={(e) => set("artcc", e.target.value)}
          />
        </Field>
        <Field label="Scope (ARTCCs)" help="FCA applies only to aircraft inside these centers.">
          <Input
            className="font-mono uppercase"
            placeholder="ZDC ZTL · all"
            value={draft.scope}
            onChange={(e) => set("scope", e.target.value)}
          />
        </Field>
      </div>

      <div className="grid grid-cols-2 gap-3">
        <Field label="Min FL">
          <Input
            type="number"
            placeholder="—"
            value={draft.minFl}
            onChange={(e) => set("minFl", e.target.value)}
          />
        </Field>
        <Field label="Max FL">
          <Input
            type="number"
            placeholder="—"
            value={draft.maxFl}
            onChange={(e) => set("maxFl", e.target.value)}
          />
        </Field>
      </div>

      <div className="rounded-md border border-border/60 bg-muted/20 p-2 text-[11px] leading-snug text-muted-foreground">
        <span className="font-medium text-foreground">Route crossing</span> — only
        aircraft whose filed route crosses this line are sequenced (NASR fixes/navaids;
        airways &amp; procedures approximate by great circle).
      </div>

      <div className="flex flex-col gap-1">
        <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
          Constraint
        </span>
        <div className="flex gap-1">
          <Button
            size="sm"
            className="flex-1"
            variant={draft.mode === "rate" ? "default" : "secondary"}
            onClick={() => set("mode", "rate")}
          >
            Rate · ac/hr
          </Button>
          <Button
            size="sm"
            className="flex-1"
            variant={draft.mode === "mit" ? "default" : "secondary"}
            onClick={() => set("mode", "mit")}
          >
            MIT · nm
          </Button>
        </div>
        {draft.mode === "rate" ? (
          <>
            <Input
              type="number"
              min={0}
              max={240}
              value={draft.rate}
              onChange={(e) => set("rate", Number(e.target.value) || 0)}
            />
            <span className="text-[11px] text-muted-foreground/80">
              aircraft per hour → fixed time spacing (MINIT) between crossings.
            </span>
          </>
        ) : (
          <>
            <Input
              type="number"
              min={0}
              max={200}
              value={draft.mit}
              onChange={(e) => set("mit", Number(e.target.value) || 0)}
            />
            <span className="text-[11px] text-muted-foreground/80">
              miles-in-trail → spacing scaled by each aircraft&apos;s crossing speed.
            </span>
          </>
        )}
      </div>

      <div className="flex flex-col gap-1">
        <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
          Color
        </span>
        <div className="flex flex-wrap gap-1.5">
          {FCA_COLORS.map((c) => (
            <button
              key={c}
              type="button"
              onClick={() => set("color", c)}
              className={
                "size-6 rounded-full " +
                (draft.color === c
                  ? "ring-2 ring-ring ring-offset-2 ring-offset-background"
                  : "")
              }
              style={{ background: c }}
            />
          ))}
        </div>
      </div>

      <div className="mt-auto flex flex-col gap-2 pt-2">
        <div className="flex gap-2">
          <Button variant="secondary" className="flex-1" onClick={onRedraw}>
            ↻ Redraw line
          </Button>
          <Button
            className="flex-1"
            onClick={onSave}
            disabled={draft.points.length < 2 || saving}
          >
            Save FCA
          </Button>
        </div>
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </div>
  );
}
