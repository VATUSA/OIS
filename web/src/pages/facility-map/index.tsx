import {useCallback, useEffect, useMemo, useState} from "react";
import {WebMercatorViewport, type MapViewState} from "@deck.gl/core";
import {useNavigate, useParams, useSearch} from "@tanstack/react-router";
import {useTheme, useToast} from "@ois/ui";
import {Code2, Maximize2, Pencil, RadioTower, Route} from "lucide-react";

import {useFacilities} from "@/lib/admin";
import {useAtc, useTraffic} from "@/lib/fca";
import {useRoutes} from "@/lib/route";
import {useFacilityMapConfig, type UpsertFacilityMapConfig} from "@/lib/facility-map";
import {buildColorFn} from "@/lib/facility-map/rules";
import {colorLabel} from "@/lib/facility-map/palette";
import {RuleEditor} from "@/components/facility-map/RuleEditor";
import {
  ALL_BOUNDARIES,
  BOUNDARY_IDS,
  facilityCollection,
  facilityFeature,
  facilityPoints,
} from "@/lib/facility-map/boundary";
import {TrafficMap} from "@/components/map/TrafficMap";
import type {AtcData} from "@/components/map/layers/atc";
import type {NamedRoute} from "@/components/map/layers/routes";
import {useMapCamera} from "@/components/map/hooks/useMapCamera";
import {aircraftColor} from "@/components/map/lib/colors";
import {US_HOME} from "@/components/map/lib/constants";
import type {NormAircraft} from "@/components/map/lib/types";

/** Read-once localStorage boolean, defaulting to `def` when unset/blocked. */
function storedBool(key: string, def: boolean): boolean {
  try {
    const v = localStorage.getItem(key);
    return v == null ? def : v === "1";
  } catch {
    return def;
  }
}

const INITIAL = { longitude: US_HOME.longitude, latitude: US_HOME.latitude, zoom: 3.9 };
/**
 * The national overview frames the CONUS, not every ARTCC vertex — the oceanic ARTCCs (Anchorage,
 * Oakland/New York Oceanic) span the Pacific/Atlantic and would shrink the mainland to a dot.
 */
const CONUS_BOUNDS: [number, number][] = [
  [-127, 23],
  [-66, 50],
];

/**
 * The starting camera framed on a set of [lon, lat] points, solved against a nominal 16:9 viewport.
 * Used as the map's INITIAL so it renders pre-framed — the post-mount `fitBounds` transition is
 * unreliable in the full-height embed (the map mounts before layout settles), so we don't depend on it.
 */
function viewForPoints(pts: [number, number][], maxZoom: number): MapViewState {
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
  try {
    const vp = new WebMercatorViewport({ width: 1200, height: 700 });
    const { longitude, latitude, zoom } = vp.fitBounds(
      [
        [minLon, minLat],
        [maxLon, maxLat],
      ],
      { padding: 40 },
    );
    return { longitude, latitude, zoom: Math.min(zoom, maxZoom) };
  } catch {
    return INITIAL;
  }
}

/** `/facility-map` — the national overview (all facilities), auto-centered on the CONUS. */
export function FacilityMapIndexPage() {
  return <FacilityMapView id={null} />;
}

/**
 * The public per-facility TMU map (`/facility-map/$facilityId`, e.g. ZDC). Shows all VATSIM traffic
 * centered on the facility's airspace, colored by that facility's rules. Anyone can view; editing the
 * rules (Phase 3) is gated on the server `editable` flag.
 */
export function FacilityMapPage() {
  const { facilityId } = useParams({ from: "/facility-map/$facilityId" });
  const { embed, atc, routes, theme } = useSearch({ from: "/facility-map/$facilityId" });
  return (
    <FacilityMapView
      id={facilityId.toUpperCase()}
      embed={embed}
      initialAtc={atc}
      initialRoutes={routes}
      forceTheme={theme}
    />
  );
}

/**
 * The map body, shared by the standalone page and the dashboard "facility map" widget. `embed` gives
 * the minimal chrome (map + aircraft + legend only) used by both external iframes and the widget.
 */
export function FacilityMapView({
  id,
  embed = false,
  initialAtc = false,
  initialRoutes = false,
  forceTheme,
}: {
  id: string | null;
  /** Embed mode: minimal chrome (map + aircraft + legend only); layers driven by props, not toggles. */
  embed?: boolean;
  initialAtc?: boolean;
  initialRoutes?: boolean;
  forceTheme?: "light" | "dark";
}) {
  const { resolvedTheme, setTheme } = useTheme();
  const navigate = useNavigate();
  const toast = useToast();

  // Embeds may pin the host page's theme via `?theme=`.
  useEffect(() => {
    if (embed && forceTheme) setTheme(forceTheme);
  }, [embed, forceTheme, setTheme]);

  // Start the camera already framed on the facility (or CONUS) so the map renders in place without a
  // post-mount transition. Computed once at mount from the id; facility switches re-fit via the effect.
  const initialView = useMemo(() => {
    const f = id ? facilityFeature(id) : null;
    return f ? viewForPoints(facilityPoints(f), 8) : INITIAL;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const camera = useMapCamera(initialView);
  const traffic = useTraffic();
  const config = useFacilityMapConfig(id);
  const facilities = useFacilities();

  // Overlay layers. Normal mode: user-toggled, default on ("the map has everything"), remembered per
  // browser. Embed mode: fixed by the URL params, and we never touch the viewer's saved prefs.
  const [atcPref, setAtcPref] = useState(() => storedBool("facilityMap.atc", true));
  const [routesPref, setRoutesPref] = useState(() => storedBool("facilityMap.routes", true));
  const showAtc = embed ? initialAtc : atcPref;
  const showRoutes = embed ? initialRoutes : routesPref;
  useEffect(() => {
    if (embed) return;
    try {
      localStorage.setItem("facilityMap.atc", atcPref ? "1" : "0");
    } catch {
      /* non-fatal */
    }
  }, [atcPref, embed]);
  useEffect(() => {
    if (embed) return;
    try {
      localStorage.setItem("facilityMap.routes", routesPref ? "1" : "0");
    } catch {
      /* non-fatal */
    }
  }, [routesPref, embed]);
  const atc = useAtc(showAtc);
  const routes = useRoutes();

  const [editing, setEditing] = useState(false);
  const [preview, setPreview] = useState<UpsertFacilityMapConfig | null>(null);
  const handlePreview = useCallback((d: UpsertFacilityMapConfig) => setPreview(d), []);

  const feature = useMemo(() => (id ? facilityFeature(id) : null), [id]);
  // A single facility's outline (emphasized) when selected; otherwise every ARTCC, faint.
  const boundaries = feature ? facilityCollection(feature) : ALL_BOUNDARIES;

  // Leave edit mode when switching facilities.
  useEffect(() => {
    setEditing(false);
    setPreview(null);
  }, [id]);

  // While editing, color from the unsaved draft (live preview); otherwise the saved config.
  const activeConfig = editing && preview ? preview : config.data;

  // Auto-center: frame the selected facility, or the whole CONUS on the national overview. Fit on
  // facility change, and once more after the canvas settles — in the full-height embed the map mounts
  // before layout finishes, so the first fit can solve against a stale size.
  useEffect(() => {
    const pts = feature ? facilityPoints(feature) : CONUS_BOUNDS;
    const opts = { padding: 40, maxZoom: feature ? 8 : 6 };
    camera.fitBounds(pts, opts);
    const t = setTimeout(() => camera.fitBounds(pts, opts), 400);
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id, feature]);

  const aircraft = useMemo<NormAircraft[]>(
    () =>
      (traffic.data ?? []).map((a) => ({
        id: a.callsign,
        callsign: a.callsign,
        actype: a.actype,
        dep: a.dep,
        arr: a.arr,
        lat: a.lat,
        lon: a.lon,
        alt: a.alt,
        gs: a.gs,
        heading: a.heading,
        star: a.star,
        wake: a.wake,
        flightRules: a.flight_rules,
        filedAlt: a.filed_alt,
      })),
    [traffic.data],
  );

  const getAircraftColor = useMemo(
    () => buildColorFn(activeConfig, aircraftColor(resolvedTheme)),
    [activeConfig, resolvedTheme],
  );

  // ARTCCs that have both a VATUSA directory entry and a boundary polygon, sorted by id.
  const pickable = useMemo(
    () =>
      (facilities.data ?? [])
        .filter((f) => BOUNDARY_IDS.has(f.id))
        .sort((a, b) => a.id.localeCompare(b.id)),
    [facilities.data],
  );

  const legendRules = (activeConfig?.rules ?? []).filter((r) => r.enabled && r.conditions.length > 0);
  const canEdit = !!feature && !!config.data?.editable;

  // Copy an <iframe> embed snippet for this facility, carrying the layers currently toggled on.
  const copyEmbed = () => {
    const params = new URLSearchParams({ embed: "1" });
    if (showAtc) params.set("atc", "1");
    if (showRoutes) params.set("routes", "1");
    const url = `${window.location.origin}/facility-map/${id}?${params.toString()}`;
    const snippet = `<iframe src="${url}" width="800" height="500" style="border:0" title="${id} traffic map" loading="lazy"></iframe>`;
    navigator.clipboard
      .writeText(snippet)
      .then(() => toast.success("Embed code copied to your clipboard"))
      .catch(() => toast.error("Couldn’t copy — check clipboard permissions"));
  };

  return (
    <div className={`relative w-full ${embed ? "h-full" : "h-[calc(100vh-3.5rem)]"}`}>
      <TrafficMap
        className="absolute inset-0"
        camera={camera}
        boundaries={boundaries}
        boundaryEmphasis={!!feature}
        aircraft={aircraft}
        getAircraftColor={getAircraftColor}
        atc={showAtc ? ((atc.data as AtcData | undefined) ?? null) : null}
        namedRoutes={showRoutes ? (routes.data as NamedRoute[] | undefined) : undefined}
        baseCursor="grab"
      >
        {/* Controls (hidden in embed mode — layers come from the URL there). */}
        {!embed && (
        <div className="absolute left-3 top-3 z-[500] flex flex-wrap items-center gap-2">
          <select
            value={id ?? ""}
            onChange={(e) =>
              navigate({ to: "/facility-map/$facilityId", params: { facilityId: e.target.value } })
            }
            title="Select a facility"
            className="rounded-lg border bg-background/95 px-2.5 py-1.5 text-xs font-medium shadow-lg backdrop-blur outline-none"
          >
            <option value="" disabled>
              Select a facility…
            </option>
            {id && !pickable.some((f) => f.id === id) && <option value={id}>{id}</option>}
            {pickable.map((f) => (
              <option key={f.id} value={f.id}>
                {f.id} — {f.name}
              </option>
            ))}
          </select>
          <button
            type="button"
            onClick={() =>
              camera.fitBounds(feature ? facilityPoints(feature) : CONUS_BOUNDS, {
                padding: 40,
                maxZoom: feature ? 8 : 6,
              })
            }
            title={feature ? "Recenter on the facility" : "Recenter on the CONUS"}
            className="flex items-center gap-1.5 rounded-lg border bg-background/95 px-2.5 py-1.5 text-xs font-medium shadow-lg backdrop-blur transition-colors hover:bg-muted"
          >
            <Maximize2 className="size-3.5 text-muted-foreground" />
            Recenter
          </button>
          <button
            type="button"
            onClick={() => setAtcPref((v) => !v)}
            title="Toggle online ATC (positions, approach & center areas)"
            aria-pressed={showAtc}
            className={`flex items-center gap-1.5 rounded-lg border px-2.5 py-1.5 text-xs font-medium shadow-lg backdrop-blur transition-colors ${
              showAtc ? "border-primary/60 bg-primary/15 text-primary" : "bg-background/95 hover:bg-muted"
            }`}
          >
            <RadioTower className={`size-3.5 ${showAtc ? "text-primary" : "text-muted-foreground"}`} />
            ATC
          </button>
          <button
            type="button"
            onClick={() => setRoutesPref((v) => !v)}
            title="Toggle saved routes"
            aria-pressed={showRoutes}
            className={`flex items-center gap-1.5 rounded-lg border px-2.5 py-1.5 text-xs font-medium shadow-lg backdrop-blur transition-colors ${
              showRoutes ? "border-primary/60 bg-primary/15 text-primary" : "bg-background/95 hover:bg-muted"
            }`}
          >
            <Route className={`size-3.5 ${showRoutes ? "text-primary" : "text-muted-foreground"}`} />
            Routes
          </button>
          {canEdit && !editing && (
            <button
              type="button"
              onClick={() => setEditing(true)}
              title="Edit this facility's color rules"
              className="flex items-center gap-1.5 rounded-lg border bg-background/95 px-2.5 py-1.5 text-xs font-medium shadow-lg backdrop-blur transition-colors hover:bg-muted"
            >
              <Pencil className="size-3.5 text-muted-foreground" />
              Edit rules
            </button>
          )}
          {feature && (
            <button
              type="button"
              onClick={copyEmbed}
              title="Copy an <iframe> snippet to embed this map (current layers included)"
              className="flex items-center gap-1.5 rounded-lg border bg-background/95 px-2.5 py-1.5 text-xs font-medium shadow-lg backdrop-blur transition-colors hover:bg-muted"
            >
              <Code2 className="size-3.5 text-muted-foreground" />
              Embed
            </button>
          )}
        </div>
        )}

        {/* Legend */}
        {legendRules.length > 0 && (
          <div className="absolute bottom-3 left-3 z-[500] max-w-[16rem] rounded-lg border bg-background/95 p-3 text-xs shadow-lg backdrop-blur">
            <div className="mb-1.5 font-semibold">{id} coloring</div>
            <ul className="flex flex-col gap-1">
              {legendRules.map((r) => (
                <li key={r.id} className="flex items-center gap-2">
                  <span
                    className="inline-block size-3 shrink-0 rounded-sm"
                    style={{ backgroundColor: r.color }}
                    title={colorLabel(r.color)}
                  />
                  <span className="truncate">{r.label || colorLabel(r.color)}</span>
                </li>
              ))}
            </ul>
          </div>
        )}

        {/* Unknown facility id in the URL (the hint points at the picker, so skip it in embed) */}
        {id && !feature && !embed && (
          <div className="pointer-events-none absolute inset-x-0 top-20 z-[400] flex justify-center">
            <div className="rounded-lg border bg-background/95 px-4 py-2 text-sm shadow-lg backdrop-blur">
              No boundary on file for <span className="font-semibold">{id}</span> — pick a facility above.
            </div>
          </div>
        )}

        {/* Rules editor (staff only) */}
        {editing && id && config.data && (
          <RuleEditor
            facilityId={id}
            initial={config.data}
            onPreview={handlePreview}
            onClose={() => {
              setEditing(false);
              setPreview(null);
            }}
          />
        )}
      </TrafficMap>
    </div>
  );
}
