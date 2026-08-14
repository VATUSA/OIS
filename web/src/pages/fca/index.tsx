import {useCallback, useEffect, useMemo, useRef, useState} from "react";
import L from "leaflet";
import "leaflet/dist/leaflet.css";
import {Button, Input, useTheme} from "@ois/ui";
import {Pencil, Plus, Trash2, X} from "lucide-react";

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
  useDeleteFca,
  useFcaCounts,
  useFcas,
  useFcaTraffic,
  useTraffic,
  useUpdateFca,
} from "@/lib/fca";
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

  const [draft, setDraft] = useState<Draft | null>(null);
  const [phase, setPhase] = useState<Phase>("draw");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [routeCallsign, setRouteCallsign] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [artccFilter, setArtccFilter] = useState("");

  const fcaTraffic = useFcaTraffic(draft ? null : selectedId);
  const counts = useFcaCounts();
  const aircraftRoute = useAircraftRoute(routeCallsign);

  const { resolvedTheme } = useTheme();

  // --- Leaflet refs ---
  const mapRef = useRef<L.Map | null>(null);
  const tileRef = useRef<L.TileLayer | null>(null);
  const roRef = useRef<ResizeObserver | null>(null);
  // Flips true once the map + layer groups exist, so the drawing effects below re-run and
  // paint even when their data resolved before the map mounted.
  const [mapReady, setMapReady] = useState(false);
  const aircraftLayer = useRef<L.LayerGroup | null>(null);
  const fcaLayer = useRef<L.LayerGroup | null>(null);
  const matchedLayer = useRef<L.LayerGroup | null>(null);
  const routeLayer = useRef<L.LayerGroup | null>(null);
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
      worldCopyJump: false,
      doubleClickZoom: false,
    }).setView([38.5, -77], 6);
    L.control.zoom({ position: "topright" }).addTo(map);
    node.style.background = MAP_BG[resolvedTheme];
    tileRef.current = L.tileLayer(CARTO[resolvedTheme], {
      maxZoom: 14,
      attribution:
        "© OpenStreetMap, © CARTO · traffic: VATSIM · boundaries: FAA NASR / ERAM",
    }).addTo(map);

    // ARTCC boundary outlines (below everything else) + faded center labels.
    L.geoJSON(boundariesGeo as GeoJSON.GeoJsonObject, {
      style: { color: "#64748b", weight: 1, opacity: 0.4, fill: false },
      interactive: false,
    }).addTo(map);
    for (const feat of (boundariesGeo as GeoJSON.FeatureCollection).features) {
      const geom = feat.geometry;
      if (geom.type !== "Polygon") continue;
      const ring = geom.coordinates[0];
      let sx = 0;
      let sy = 0;
      for (const [lon, lat] of ring) {
        sx += lon;
        sy += lat;
      }
      const c: LatLng = [sy / ring.length, sx / ring.length];
      L.marker(c, {
        icon: L.divIcon({
          className: "",
          html: `<span style="color:#64748b;font:600 11px ui-monospace,monospace;opacity:.5">${feat.properties?.id ?? ""}</span>`,
          iconSize: [0, 0],
        }),
        interactive: false,
        keyboard: false,
      }).addTo(map);
    }

    routeLayer.current = L.layerGroup().addTo(map);
    fcaLayer.current = L.layerGroup().addTo(map);
    aircraftLayer.current = L.layerGroup().addTo(map);
    matchedLayer.current = L.layerGroup().addTo(map);
    draftLayer.current = L.layerGroup().addTo(map);

    map.on("click", (e: L.LeafletMouseEvent) => {
      if (!drawingRef.current) return;
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

  // Live aircraft — hover for details, click to plot the route. Aircraft matched to
  // the selected FCA are drawn (tinted + numbered) by the matched layer instead.
  useEffect(() => {
    const layer = aircraftLayer.current;
    if (!layer) return;
    layer.clearLayers();
    const matched = new Set((fcaTraffic.data ?? []).map((f) => f.callsign));
    for (const ac of traffic.data ?? []) {
      if (matched.has(ac.callsign)) continue;
      L.marker([ac.lat, ac.lon], {
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
  }, [traffic.data, fcaTraffic.data, mapReady]);

  // Plotted route for a clicked aircraft.
  useEffect(() => {
    const layer = routeLayer.current;
    if (!layer) return;
    layer.clearLayers();
    const pts = aircraftRoute.data?.points as LatLng[] | undefined;
    if (pts && pts.length >= 2) {
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
      // Label each named waypoint along the route.
      for (const wp of aircraftRoute.data?.waypoints ?? []) {
        L.marker([wp.lat, wp.lon], {
          icon: waypointLabelIcon(wp.name),
          interactive: false,
          keyboard: false,
          zIndexOffset: -200,
        }).addTo(layer);
      }
    }
  }, [aircraftRoute.data, mapReady]);

  // Saved FCAs (skip the one being edited — drawn on the draft layer).
  useEffect(() => {
    const layer = fcaLayer.current;
    if (!layer) return;
    layer.clearLayers();
    for (const fca of fcas.data ?? []) {
      if (fca.id === draft?.id) continue;
      const pts = fca.points as LatLng[];
      if (!pts || pts.length < 2) continue;
      const selected = fca.id === selectedId;
      const opacity = fca.enabled ? (selected ? 1 : 0.85) : 0.3;
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
      // Filled colour dots at the first and last point.
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
  }, [fcas.data, draft?.id, selectedId, mapReady]);

  // Matched (crossing) traffic for the selected FCA — numbered, in the FCA colour.
  const selectedColor = fcas.data?.find((f) => f.id === selectedId)?.color;
  useEffect(() => {
    const layer = matchedLayer.current;
    if (!layer) return;
    layer.clearLayers();
    if (draft || !selectedColor) return;
    const color = selectedColor;
    for (const f of fcaTraffic.data ?? []) {
      const hasPos = f.lat !== 0 || f.lon !== 0;
      if (hasPos) {
        // Full resolved route (remaining route for airborne), in the FCA colour.
        const path = f.path as LatLng[] | undefined;
        const line =
          path && path.length >= 2
            ? path
            : ([
                [f.lat, f.lon],
                [f.cross_lat, f.cross_lon],
              ] as LatLng[]);
        L.polyline(line, {
          color,
          weight: 1.5,
          opacity: 0.55,
          interactive: false,
        }).addTo(layer);
      }
      L.circleMarker([f.cross_lat, f.cross_lon], {
        radius: 3,
        color: "#ffffff",
        weight: 1,
        fillColor: "#ffffff",
        fillOpacity: 0.9,
        interactive: false,
      }).addTo(layer);
      if (hasPos) {
        L.marker([f.lat, f.lon], {
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
  }, [fcaTraffic.data, draft, selectedColor, mapReady]);

  // Working draft (dashed polyline + draggable vertex handles).
  useEffect(() => {
    const layer = draftLayer.current;
    if (!layer) return;
    layer.clearLayers();
    if (!draft) return;
    if (draft.points.length >= 2) {
      L.polyline(draft.points, {
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
  const cancel = () => setDraft(null);

  const save = () => {
    if (!draft || draft.points.length < 2) return;
    const body: UpsertFca = {
      name: draft.name.trim() || "FCA",
      color: draft.color,
      artcc: draft.artcc.trim().toUpperCase(),
      points: draft.points,
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
                        <button
                          type="button"
                          title="Delete"
                          onClick={() => deleteFca.mutate(fca.id)}
                          className="text-muted-foreground hover:text-destructive"
                        >
                          <Trash2 className="size-3.5" />
                        </button>
                      )}
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </>
        )}

        <div className="border-t px-4 py-2 text-xs text-muted-foreground">
          {traffic.data ? `traffic ${traffic.data.length}` : "traffic…"}
        </div>
      </aside>

      {/* Map — `isolate` traps Leaflet z-indexes below the navbar dropdowns. */}
      <div className="relative isolate flex-1">
        <div ref={setContainer} className="absolute inset-0" />
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
  return (
    <div className="absolute left-3 top-3 z-[600] w-[min(92vw,26rem)] rounded-xl border border-border/70 bg-background/95 p-4 shadow-2xl backdrop-blur">
      <div className="flex items-start justify-between gap-3">
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
        <button
          type="button"
          onClick={onClose}
          className="-mr-1 -mt-1 rounded p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
          aria-label="Close"
        >
          <X className="size-4" />
        </button>
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
