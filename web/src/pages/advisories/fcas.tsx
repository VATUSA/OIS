import {useCallback, useEffect, useRef, useState} from "react";
import L from "leaflet";
import "leaflet/dist/leaflet.css";
import {Badge, useTheme} from "@ois/ui";
import {Link} from "@tanstack/react-router";
import {ArrowLeft, Waypoints} from "lucide-react";

import {type PublicFca, usePublicFcas} from "@/lib/public";
import boundariesGeo from "@/assets/artcc-boundaries.json";

const CARTO = {
  dark: "https://{s}.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}{r}.png",
  light: "https://{s}.basemaps.cartocdn.com/light_all/{z}/{x}/{y}{r}.png",
} as const;
const MAP_BG = { dark: "#0a0a0a", light: "#e5e7eb" } as const;
const US_HOME = { center: [39.5, -98.35] as [number, number], zoom: 4.3 };

type LatLng = [number, number];

/** Middle-ish vertex of a polyline, for placing the name label. */
function midpoint(pts: LatLng[]): LatLng {
  return pts[Math.floor(pts.length / 2)] ?? pts[0];
}

/** "FL240–390" / "≤FL390" / "≥FL240" / null. */
function flBand(
  min: number | null | undefined,
  max: number | null | undefined,
): string | null {
  if (min != null && max != null) return `FL${min}–${max}`;
  if (max != null) return `≤FL${max}`;
  if (min != null) return `≥FL${min}`;
  return null;
}

const DIR_LABEL: Record<string, string> = {
  N: "NB",
  S: "SB",
  E: "EB",
  W: "WB",
};

function fcaLabelIcon(name: string, color: string) {
  return L.divIcon({
    className: "",
    html: `<span style="display:inline-block;font:600 11px ui-monospace,monospace;color:${color};white-space:nowrap;text-shadow:0 0 3px #000,0 1px 2px #000;transform:translate(6px,-50%)">${name}</span>`,
    iconSize: [0, 0],
    iconAnchor: [0, 0],
  });
}

/** One FCA's read-only detail card in the sidebar. */
function FcaCard({
  fca,
  onFocus,
}: {
  fca: PublicFca;
  onFocus: (fca: PublicFca) => void;
}) {
  const constraint =
    fca.mode === "mit" ? `${fca.mit} MIT` : `${fca.rate}/hr`;
  const fl = flBand(fca.min_fl, fca.max_fl);
  return (
    <button
      type="button"
      onClick={() => onFocus(fca)}
      className="flex w-full flex-col gap-1.5 rounded-lg border px-3 py-2.5 text-left transition-colors hover:bg-muted"
    >
      <div className="flex items-center gap-2">
        <span
          className="size-2.5 shrink-0 rounded-full"
          style={{ background: fca.color }}
        />
        <span className="text-sm font-semibold">{fca.name}</span>
        <span className="ml-auto font-mono text-xs text-muted-foreground">
          {fca.artcc}
        </span>
      </div>
      <div className="flex flex-wrap items-center gap-1.5">
        <Badge variant="secondary">{constraint}</Badge>
        {fca.dir !== "any" && (
          <Badge variant="outline">{DIR_LABEL[fca.dir] ?? fca.dir}</Badge>
        )}
        {fl && <Badge variant="outline">{fl}</Badge>}
      </div>
      {(fca.dests.length > 0 ||
        fca.origins.length > 0 ||
        fca.fixes.length > 0) && (
        <div className="flex flex-col gap-0.5 font-mono text-xs text-muted-foreground">
          {fca.dests.length > 0 && <span>→ {fca.dests.join(" ")}</span>}
          {fca.origins.length > 0 && <span>from {fca.origins.join(" ")}</span>}
          {fca.fixes.length > 0 && <span>via {fca.fixes.join(" ")}</span>}
        </div>
      )}
    </button>
  );
}

export function AdvisoriesFcaPage() {
  const { data: fcas } = usePublicFcas();
  const { resolvedTheme } = useTheme();

  const mapRef = useRef<L.Map | null>(null);
  const tileRef = useRef<L.TileLayer | null>(null);
  const boundaryLayer = useRef<L.LayerGroup | null>(null);
  const fcaLayer = useRef<L.LayerGroup | null>(null);
  const roRef = useRef<ResizeObserver | null>(null);
  const [mapReady, setMapReady] = useState(false);

  const setContainer = useCallback((node: HTMLDivElement | null) => {
    if (!node) {
      roRef.current?.disconnect();
      mapRef.current?.remove();
      mapRef.current = null;
      tileRef.current = null;
      setMapReady(false);
      return;
    }
    if (mapRef.current) return;
    const map = L.map(node, {
      zoomControl: false,
      preferCanvas: true,
      zoomSnap: 0,
      zoomDelta: 0.5,
      wheelPxPerZoomLevel: 36,
      wheelDebounceTime: 10,
    }).setView(US_HOME.center, US_HOME.zoom);
    L.control.zoom({ position: "topright" }).addTo(map);
    node.style.background = MAP_BG[resolvedTheme];
    tileRef.current = L.tileLayer(CARTO[resolvedTheme], {
      maxZoom: 14,
      attribution: "© OpenStreetMap, © CARTO · boundaries: FAA NASR / ERAM",
    }).addTo(map);
    boundaryLayer.current = L.layerGroup().addTo(map);
    fcaLayer.current = L.layerGroup().addTo(map);

    // ARTCC boundaries — faint reference outlines, drawn once.
    const fc = boundariesGeo as GeoJSON.FeatureCollection;
    for (const feat of fc.features) {
      const geom = feat.geometry;
      const rings =
        geom.type === "Polygon"
          ? geom.coordinates
          : geom.type === "MultiPolygon"
            ? geom.coordinates.flat()
            : [];
      for (const ring of rings) {
        L.polyline(
          ring.map(([lon, lat]) => [lat, lon] as LatLng),
          { color: "#64748b", weight: 1, opacity: 0.35, interactive: false },
        ).addTo(boundaryLayer.current!);
      }
    }

    mapRef.current = map;
    const ro = new ResizeObserver(() => map.invalidateSize());
    ro.observe(node);
    roRef.current = ro;
    map.invalidateSize();
    setMapReady(true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Swap tiles on theme change.
  useEffect(() => {
    tileRef.current?.setUrl(CARTO[resolvedTheme]);
    const c = mapRef.current?.getContainer();
    if (c) c.style.background = MAP_BG[resolvedTheme];
  }, [resolvedTheme]);

  // Draw the FCAs (non-interactive polylines + name labels).
  useEffect(() => {
    const layer = fcaLayer.current;
    if (!layer) return;
    layer.clearLayers();
    for (const fca of fcas ?? []) {
      const pts = fca.points as LatLng[];
      if (!pts || pts.length < 2) continue;
      const tip =
        `<b>${fca.name}</b> · ${fca.artcc}<br>` +
        (fca.mode === "mit" ? `${fca.mit} MIT` : `${fca.rate}/hr`);
      L.polyline(pts, {
        color: fca.color,
        weight: 3,
        opacity: 0.9,
      })
        .bindTooltip(tip, { sticky: true, className: "fca-tip" })
        .addTo(layer);
      L.marker(midpoint(pts), {
        icon: fcaLabelIcon(fca.name, fca.color),
        interactive: false,
        keyboard: false,
      }).addTo(layer);
    }
  }, [fcas, mapReady]);

  const focusFca = (fca: PublicFca) => {
    const pts = fca.points as LatLng[];
    if (mapRef.current && pts?.length) {
      mapRef.current.flyToBounds(L.latLngBounds(pts).pad(0.5), {
        duration: 0.6,
      });
    }
  };

  const list = fcas ?? [];

  return (
    <div className="flex h-[calc(100vh-3.5rem)]">
      <aside className="flex w-80 shrink-0 flex-col border-r">
        <div className="flex items-center gap-2 border-b px-4 py-3">
          <Waypoints className="size-4 text-primary" />
          <h1 className="text-sm font-semibold">Flow Constrained Areas</h1>
          <span className="ml-auto text-sm text-muted-foreground tabular-nums">
            {list.length}
          </span>
        </div>
        <Link
          to="/advisories"
          className="flex items-center gap-1.5 border-b px-4 py-2 text-xs text-muted-foreground transition-colors hover:text-foreground"
        >
          <ArrowLeft className="size-3.5" />
          Back to advisories
        </Link>
        <div className="flex flex-1 flex-col gap-2 overflow-y-auto p-3">
          {list.length === 0 ? (
            <p className="py-10 text-center text-sm text-muted-foreground">
              No active FCAs.
            </p>
          ) : (
            list.map((fca) => (
              <FcaCard key={fca.id} fca={fca} onFocus={focusFca} />
            ))
          )}
        </div>
      </aside>
      <div className="relative isolate flex-1">
        <div ref={setContainer} className="absolute inset-0" />
      </div>
    </div>
  );
}
