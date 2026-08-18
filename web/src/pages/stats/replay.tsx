import {useEffect, useMemo, useRef, useState} from "react";
import DeckGL from "@deck.gl/react";
import {GeoJsonLayer, IconLayer, PathLayer, TextLayer} from "@deck.gl/layers";
import type {PickingInfo} from "@deck.gl/core";
import {Map as MapLibre} from "react-map-gl/maplibre";
import "maplibre-gl/dist/maplibre-gl.css";
import {Button, useTheme} from "@ois/ui";
import {Link, useNavigate, useSearch} from "@tanstack/react-router";
import {ArrowLeft, Pause, Play, SkipBack, X} from "lucide-react";

import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {type Replay, useCaptureReplay, useCaptures, useWindowReplay} from "@/lib/stats";
import {aircraftIconUrl} from "@/lib/aircraft-icons";
import {formatZuluFull} from "@/lib/time";
import boundariesGeo from "@/assets/artcc-boundaries.json";

// Free CARTO vector basemap styles (no access token needed).
const CARTO_STYLE = {
  dark: "https://basemaps.cartocdn.com/gl/dark-matter-gl-style/style.json",
  light: "https://basemaps.cartocdn.com/gl/positron-gl-style/style.json",
} as const;
const INITIAL_VIEW = { longitude: -98.35, latitude: 39.5, zoom: 3.4 };

// Per-theme colors for the airport-layout overlay: apron/surface fill, taxiway lines, runway lines.
const AEROWAY_COLORS = {
  dark: { fill: "#20242e", taxiway: "#4a5162", runway: "#8a93a6" },
  light: { fill: "#e3e7ee", taxiway: "#c4cad4", runway: "#98a1b2" },
} as const;

/** Minimal MapLibre surface we touch — avoids depending on maplibre-gl's exported types. */
interface StyleMap {
  getSource(id: string): unknown;
  getLayer(id: string): unknown;
  addLayer(layer: Record<string, unknown>): void;
}

/**
 * Draw airport layouts (runways, taxiways, aprons) straight from the OSM `aeroway` data already in
 * the CARTO vector tiles — the base style renders it in near-black (invisible), so we add our own
 * visible layers instead. Free, no extra requests, appears once you zoom into a field (z≥10).
 * Idempotent: safe to call on every `styledata` (re-added after a theme swap wipes the style).
 */
function ensureAeroway(map: StyleMap, theme: "dark" | "light"): void {
  try {
    if (!map.getSource("carto") || map.getLayer("ois-aeroway-fill")) return;
    const c = AEROWAY_COLORS[theme];
    const base = { source: "carto", "source-layer": "aeroway" } as const;
    map.addLayer({
      ...base,
      id: "ois-aeroway-fill",
      type: "fill",
      minzoom: 10,
      filter: ["==", ["geometry-type"], "Polygon"],
      paint: { "fill-color": c.fill, "fill-opacity": 0.6 },
    });
    map.addLayer({
      ...base,
      id: "ois-aeroway-taxiway",
      type: "line",
      minzoom: 12,
      filter: ["all", ["==", ["geometry-type"], "LineString"], ["==", ["get", "class"], "taxiway"]],
      paint: {
        "line-color": c.taxiway,
        "line-width": ["interpolate", ["linear"], ["zoom"], 12, 0.6, 14, 1.5, 16, 4],
      },
    });
    map.addLayer({
      ...base,
      id: "ois-aeroway-runway",
      type: "line",
      minzoom: 10,
      filter: ["all", ["==", ["geometry-type"], "LineString"], ["==", ["get", "class"], "runway"]],
      paint: {
        "line-color": c.runway,
        "line-width": ["interpolate", ["linear"], ["zoom"], 10, 1.2, 13, 4, 15, 9, 16, 13],
      },
    });
  } catch {
    // Style not fully ready yet — a later `styledata` event retries.
  }
}
const SPEEDS = [1, 2, 4, 8, 16, 32, 64];
/** Below this groundspeed an aircraft is treated as on the ground (taxi/parked). */
const GROUND_KT = 30;

/** One aircraft rendered at the current replay clock. */
type Live = {
  id: string;
  callsign: string;
  actype: string;
  dep: string;
  arr: string;
  lon: number;
  lat: number;
  alt: number;
  gs: number;
  heading: number;
};

/** A flight's samples as a sorted array of [t, lat, lon, alt, hdg, gs]. */
type Track = {
  id: string;
  callsign: string;
  actype: string;
  dep: string;
  arr: string;
  s: number[][];
};

function zulu(base: string, offsetS: number): string {
  const d = new Date(Date.parse(base) + offsetS * 1000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getUTCDate())}/${p(d.getUTCHours())}${p(d.getUTCMinutes())}${p(d.getUTCSeconds())}z`;
}

/** The `[lon,lat]` path a flight has flown up to `clock` — never its future path — extended to its
 * current interpolated position so the line reaches the aircraft. Samples are `[t,lat,lon,...]`. */
function flownPath(s: number[][], clock: number): [number, number][] {
  const pts: [number, number][] = [];
  if (s.length === 0) return pts;
  for (const p of s) {
    if (p[0] <= clock) pts.push([p[2], p[1]]);
    else break;
  }
  if (clock > s[0][0] && clock <= s[s.length - 1][0]) {
    let lo = 0;
    let hi = s.length - 1;
    while (hi - lo > 1) {
      const m = (lo + hi) >> 1;
      if (s[m][0] <= clock) lo = m;
      else hi = m;
    }
    const a = s[lo];
    const b = s[hi];
    const k = (clock - a[0]) / (b[0] - a[0] || 1);
    pts.push([a[2] + (b[2] - a[2]) * k, a[1] + (b[1] - a[1]) * k]);
  }
  return pts;
}

type Labels = { callsign: boolean; type: boolean; alt: boolean; speed: boolean };

function Toggle({
  checked,
  onChange,
  children,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  children: React.ReactNode;
}) {
  return (
    <label className="flex cursor-pointer items-center gap-2 text-sm">
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} />
      {children}
    </label>
  );
}

function ReplayMap({ replay }: { replay: Replay }) {
  const { resolvedTheme } = useTheme();

  const duration = useMemo(
    () => Math.max(1, (Date.parse(replay.window_end) - Date.parse(replay.window_start)) / 1000),
    [replay],
  );

  const tracks = useMemo<Track[]>(
    () =>
      replay.flights.map((f) => ({
        id: f.session_id,
        callsign: f.callsign,
        actype: f.aircraft ?? "",
        dep: f.departure ?? "",
        arr: f.arrival ?? "",
        s: f.samples as number[][],
      })),
    [replay],
  );

  const clockRef = useRef(0);
  const playingRef = useRef(false);
  const speedRef = useRef(1);
  const [clock, setClock] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [speed, setSpeed] = useState(1);
  const [aircraft, setAircraft] = useState<Live[]>([]);

  // Display toggles.
  const [hideGround, setHideGround] = useState(true);
  const [labels, setLabels] = useState<Labels>({ callsign: true, type: false, alt: false, speed: false });

  // Clicked flight — draws its flown-so-far track and opens the log panel.
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const selectedTrack = useMemo(
    () => tracks.find((t) => t.id === selectedId) ?? null,
    [tracks, selectedId],
  );

  // Show flown-so-far trails behind every visible aircraft (+ optionally disconnected ones).
  const [showTrails, setShowTrails] = useState(false);
  const [showDisconnected, setShowDisconnected] = useState(false);

  // The selected flight's history trail (only what it has flown at the current clock).
  const trackPath = useMemo(() => {
    if (!selectedTrack) return [];
    const pts = flownPath(selectedTrack.s, clock);
    return pts.length >= 2 ? [{ path: pts }] : [];
  }, [selectedTrack, clock]);

  // A trail for every currently-shown aircraft (when enabled). Currently-active aircraft respect the
  // ground filter; `showDisconnected` also keeps trails for flights that have flown but are no longer
  // active at the clock (landed / left the window).
  const allTrails = useMemo(() => {
    if (!showTrails) return [];
    const activeGs = new Map(aircraft.map((a) => [a.id, a.gs]));
    const out: { path: [number, number][] }[] = [];
    for (const t of tracks) {
      if (t.s.length === 0 || t.s[0][0] > clock) continue; // hasn't started yet
      const gs = activeGs.get(t.id);
      if (gs === undefined) {
        if (!showDisconnected) continue; // disconnected — only when opted in
      } else if (hideGround && gs < GROUND_KT) {
        continue;
      }
      const pts = flownPath(t.s, clock);
      if (pts.length >= 2) out.push({ path: pts });
    }
    return out;
  }, [showTrails, showDisconnected, tracks, clock, aircraft, hideGround]);

  // Log rows: samples flown so far (the history).
  const flownRows = useMemo(
    () => (selectedTrack ? selectedTrack.s.filter((p) => p[0] <= clock) : []),
    [selectedTrack, clock],
  );

  function frameAt(t: number): Live[] {
    const out: Live[] = [];
    for (const f of tracks) {
      const s = f.s;
      if (s.length === 0 || t < s[0][0] || t > s[s.length - 1][0]) continue;
      let lo = 0;
      let hi = s.length - 1;
      while (hi - lo > 1) {
        const mid = (lo + hi) >> 1;
        if (s[mid][0] <= t) lo = mid;
        else hi = mid;
      }
      const a = s[lo];
      const b = s[hi];
      const span = b[0] - a[0] || 1;
      const k = (t - a[0]) / span;
      out.push({
        id: f.id,
        callsign: f.callsign,
        actype: f.actype,
        dep: f.dep,
        arr: f.arr,
        lat: a[1] + (b[1] - a[1]) * k,
        lon: a[2] + (b[2] - a[2]) * k,
        alt: Math.round(a[3] + (b[3] - a[3]) * k),
        heading: a[4],
        gs: Math.round(a[5] + (b[5] - a[5]) * k),
      });
    }
    return out;
  }

  const render = (t: number) => {
    setAircraft(frameAt(t));
    setClock(t);
  };

  useEffect(() => {
    clockRef.current = 0;
    render(0);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tracks]);

  // Nudge deck.gl to re-measure once layout has settled (0-sized-at-mount safety).
  useEffect(() => {
    const t = setTimeout(() => window.dispatchEvent(new Event("resize")), 150);
    return () => clearTimeout(t);
  }, []);

  useEffect(() => {
    let raf = 0;
    let last = performance.now();
    let lastRender = 0;
    const loop = (now: number) => {
      const dt = (now - last) / 1000;
      last = now;
      if (playingRef.current) {
        clockRef.current = Math.min(duration, clockRef.current + dt * speedRef.current);
        if (clockRef.current >= duration) {
          playingRef.current = false;
          setPlaying(false);
        }
      }
      if (playingRef.current && now - lastRender >= 33) {
        lastRender = now;
        render(clockRef.current);
      }
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(raf);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [duration]);

  // Ground filter applied once; both the icon + label layers share the result.
  const shown = useMemo(
    () => (hideGround ? aircraft.filter((a) => a.gs >= GROUND_KT) : aircraft),
    [aircraft, hideGround],
  );

  const iconColor: [number, number, number] = resolvedTheme === "dark" ? [255, 190, 70] : [40, 60, 90];
  const HL: [number, number, number] = [56, 189, 248]; // selected flight highlight
  const boundaryColor: [number, number, number, number] =
    resolvedTheme === "dark" ? [130, 140, 160, 110] : [90, 100, 120, 120];
  const labelColor: [number, number, number] = resolvedTheme === "dark" ? [230, 235, 245] : [20, 25, 35];

  const anyLabel = labels.callsign || labels.type || labels.alt || labels.speed;

  const layers = [
    new GeoJsonLayer({
      id: "artcc-boundaries",
      data: boundariesGeo as GeoJSON.FeatureCollection,
      stroked: true,
      filled: false,
      getLineColor: boundaryColor,
      getLineWidth: 1,
      lineWidthUnits: "pixels",
      lineWidthMinPixels: 1,
    }),
    new PathLayer<{ path: [number, number][] }>({
      id: "all-trails",
      data: allTrails,
      getPath: (d) => d.path,
      getColor: [...iconColor, 80] as [number, number, number, number],
      getWidth: 1.4,
      widthUnits: "pixels",
      widthMinPixels: 1,
      updateTriggers: { getColor: [resolvedTheme] },
    }),
    new PathLayer<{ path: [number, number][] }>({
      id: "selected-track",
      data: trackPath,
      getPath: (d) => d.path,
      getColor: [...HL, 220] as [number, number, number, number],
      getWidth: 2,
      widthUnits: "pixels",
      widthMinPixels: 2,
      capRounded: true,
      jointRounded: true,
    }),
    new IconLayer<Live>({
      id: "aircraft",
      data: shown,
      pickable: true,
      getIcon: (d) => {
        const url = aircraftIconUrl(d.actype);
        return { id: url, url, width: 48, height: 48, mask: true };
      },
      getPosition: (d) => [d.lon, d.lat],
      getAngle: (d) => 360 - d.heading,
      getColor: (d) => (d.id === selectedId ? HL : iconColor),
      getSize: (d) => (d.id === selectedId ? 34 : 26),
      sizeUnits: "pixels",
      billboard: false,
      updateTriggers: { getColor: [resolvedTheme, selectedId], getSize: [selectedId] },
    }),
    new TextLayer<Live>({
      id: "labels",
      data: anyLabel ? shown : [],
      getPosition: (d) => [d.lon, d.lat],
      getText: (d) => {
        const lines: string[] = [];
        if (labels.callsign) lines.push(d.callsign);
        if (labels.type && d.actype) lines.push(d.actype);
        if (labels.alt) lines.push(`${d.alt}ft`);
        if (labels.speed) lines.push(`${d.gs}kt`);
        return lines.join("\n");
      },
      getColor: labelColor,
      getSize: 11,
      getPixelOffset: [0, 16],
      getTextAnchor: "middle",
      getAlignmentBaseline: "top",
      background: true,
      getBackgroundColor: resolvedTheme === "dark" ? [10, 12, 16, 180] : [255, 255, 255, 190],
      backgroundPadding: [3, 1],
      updateTriggers: {
        getText: [labels.callsign, labels.type, labels.alt, labels.speed],
        getColor: [resolvedTheme],
      },
    }),
  ];

  const toggle = () => {
    const next = !playingRef.current;
    if (next && clockRef.current >= duration) {
      clockRef.current = 0;
      render(0);
    }
    playingRef.current = next;
    setPlaying(next);
  };
  const scrub = (v: number) => {
    clockRef.current = v;
    render(v);
  };
  const setSpd = (v: number) => {
    speedRef.current = v;
    setSpeed(v);
  };

  const handleClick = (info: PickingInfo) => {
    const id = info.layer?.id === "aircraft" ? ((info.object as Live | undefined)?.id ?? null) : null;
    setSelectedId((prev) => (prev === id ? null : id));
  };

  const tooltip = (info: PickingInfo<Live>) => {
    const d = info.object;
    if (!d) return null;
    return {
      html:
        `<div style="font-weight:600">${d.callsign}</div>` +
        `<div>${d.dep || "????"} → ${d.arr || "????"}</div>` +
        `<div>${d.actype || "—"} · ${d.alt}ft · ${d.gs}kt</div>`,
      style: {
        background: resolvedTheme === "dark" ? "#111418" : "#ffffff",
        color: resolvedTheme === "dark" ? "#e6edf3" : "#1b1f24",
        fontSize: "12px",
        padding: "6px 8px",
        borderRadius: "6px",
        boxShadow: "0 2px 8px rgba(0,0,0,.3)",
      },
    };
  };

  return (
    <div className="flex flex-col gap-3">
      <div
        className="relative w-full overflow-hidden rounded-lg border"
        style={{ height: "70vh" }}
      >
        <DeckGL
          initialViewState={INITIAL_VIEW}
          controller
          layers={layers}
          getTooltip={tooltip}
          onClick={handleClick}
          getCursor={({ isHovering }) => (isHovering ? "pointer" : "grab")}
          style={{ position: "absolute", top: "0", left: "0", width: "100%", height: "100%" }}
        >
          <MapLibre
            mapStyle={CARTO_STYLE[resolvedTheme]}
            attributionControl={false}
            onLoad={(e) => ensureAeroway(e.target as unknown as StyleMap, resolvedTheme)}
            onStyleData={(e) => ensureAeroway(e.target as unknown as StyleMap, resolvedTheme)}
          />
        </DeckGL>

        <div className="pointer-events-none absolute left-3 top-3 z-10 rounded-md bg-background/80 px-3 py-1.5 text-sm shadow backdrop-blur">
          <span className="font-mono font-medium">{zulu(replay.window_start, clock)}</span>
          <span className="ml-2 text-muted-foreground">{shown.length} aircraft</span>
        </div>

        <div className="absolute right-3 top-3 z-10 flex flex-col gap-1.5 rounded-md border bg-background/85 px-3 py-2.5 shadow backdrop-blur">
          <Toggle checked={hideGround} onChange={setHideGround}>
            Hide aircraft on ground
          </Toggle>
          <Toggle checked={showTrails} onChange={setShowTrails}>
            Show history trails
          </Toggle>
          {showTrails && (
            <label className="ml-5 flex cursor-pointer items-center gap-2 text-sm text-muted-foreground">
              <input
                type="checkbox"
                checked={showDisconnected}
                onChange={(e) => setShowDisconnected(e.target.checked)}
              />
              Include disconnected
            </label>
          )}
          <div className="my-0.5 h-px bg-border" />
          <span className="text-xs font-medium text-muted-foreground">Labels</span>
          <Toggle checked={labels.callsign} onChange={(v) => setLabels((l) => ({ ...l, callsign: v }))}>
            Callsign
          </Toggle>
          <Toggle checked={labels.type} onChange={(v) => setLabels((l) => ({ ...l, type: v }))}>
            Aircraft type
          </Toggle>
          <Toggle checked={labels.alt} onChange={(v) => setLabels((l) => ({ ...l, alt: v }))}>
            Altitude
          </Toggle>
          <Toggle checked={labels.speed} onChange={(v) => setLabels((l) => ({ ...l, speed: v }))}>
            Groundspeed
          </Toggle>
        </div>

        {selectedTrack && (
          <div className="absolute bottom-3 left-3 z-10 flex max-h-[46%] w-80 flex-col overflow-hidden rounded-md border bg-background/90 shadow backdrop-blur">
            <div className="flex items-center justify-between gap-2 border-b px-3 py-2">
              <div className="flex flex-col">
                <span className="font-mono font-semibold" style={{ color: "rgb(56,189,248)" }}>
                  {selectedTrack.callsign}
                </span>
                <span className="text-xs text-muted-foreground">
                  {selectedTrack.dep || "????"} → {selectedTrack.arr || "????"} ·{" "}
                  {selectedTrack.actype || "—"} · {flownRows.length}/{selectedTrack.s.length} pts
                </span>
              </div>
              <button
                type="button"
                onClick={() => setSelectedId(null)}
                className="text-muted-foreground transition-colors hover:text-foreground"
                aria-label="Close track log"
              >
                <X className="size-4" />
              </button>
            </div>
            <div className="overflow-auto">
              <table className="w-full text-xs">
                <thead className="sticky top-0 bg-background/95 text-left text-muted-foreground">
                  <tr>
                    <th className="px-3 py-1 font-medium">Time</th>
                    <th className="py-1 pr-2 font-medium">Alt</th>
                    <th className="py-1 pr-2 font-medium">GS</th>
                    <th className="py-1 pr-3 font-medium">Position</th>
                  </tr>
                </thead>
                <tbody className="font-mono">
                  {flownRows.length === 0 ? (
                    <tr>
                      <td colSpan={4} className="px-3 py-2 text-muted-foreground">
                        No history yet at this time — press play or scrub forward.
                      </td>
                    </tr>
                  ) : (
                    flownRows.map((p, i) => (
                      <tr key={i} className="border-t border-border/50">
                        <td className="px-3 py-0.5">{zulu(replay.window_start, p[0])}</td>
                        <td className="py-0.5 pr-2 tabular-nums">{p[3]}</td>
                        <td className="py-0.5 pr-2 tabular-nums">{p[5]}</td>
                        <td className="py-0.5 pr-3 tabular-nums text-muted-foreground">
                          {p[1].toFixed(2)}, {p[2].toFixed(2)}
                        </td>
                      </tr>
                    ))
                  )}
                </tbody>
              </table>
            </div>
          </div>
        )}
      </div>

      <div className="flex flex-wrap items-center gap-3">
        <Button size="icon" variant="secondary" title="Restart" onClick={() => scrub(0)}>
          <SkipBack className="size-4" />
        </Button>
        <Button size="icon" onClick={toggle} title={playing ? "Pause" : "Play"}>
          {playing ? <Pause className="size-4" /> : <Play className="size-4" />}
        </Button>
        <input
          type="range"
          className="h-2 min-w-[200px] flex-1 cursor-pointer accent-primary"
          min={0}
          max={Math.floor(duration)}
          step={1}
          value={Math.floor(clock)}
          onChange={(e) => scrub(Number(e.target.value))}
        />
        <div className="flex items-center gap-1">
          {SPEEDS.map((sp) => (
            <Button
              key={sp}
              size="sm"
              variant={speed === sp ? "default" : "secondary"}
              className="h-7 px-2 tabular-nums"
              onClick={() => setSpd(sp)}
            >
              {sp}×
            </Button>
          ))}
        </div>
      </div>
    </div>
  );
}

interface ReplaySearch {
  capture?: string;
  from?: number;
  to?: number;
}

interface Win {
  from: number;
  to: number;
}

/** <input type="datetime-local"> value ↔ unix seconds (UTC/Zulu). */
const toLocalInput = (unixS: number) => {
  const d = new Date(unixS * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getUTCFullYear()}-${pad(d.getUTCMonth() + 1)}-${pad(d.getUTCDate())}T${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())}`;
};
const fromLocalInput = (v: string): number | null => {
  const ms = Date.parse(v + "Z");
  return Number.isFinite(ms) ? Math.floor(ms / 1000) : null;
};
const defaultWindow = (): Win => {
  const now = Math.floor(Date.now() / 1000);
  return { from: now - 3 * 3600, to: now };
};

/**
 * The historical replay map: pick a saved capture OR a custom time window, then play the recorded
 * traffic back and scrub through it. The selection lives in the URL (`?capture=` or `?from&to`) so a
 * replay is shareable. The heavy deck.gl `ReplayMap` is keyed on the selection so a new pick starts
 * the player fresh.
 */
export function CaptureReplayPage() {
  const { data: me } = useMe();
  const canRead = hasPermission(me, "stats.data.read");

  const search = useSearch({ strict: false }) as ReplaySearch;
  const navigate = useNavigate();
  const patch = (p: Partial<ReplaySearch>) =>
    void navigate({
      to: "/historical/replay",
      search: (prev) => ({ ...prev, ...p }),
      replace: true,
      resetScroll: false,
    });

  const captures = useCaptures();
  const [captureId, setCaptureId] = useState<string>(() => search.capture ?? "");
  const [win, setWin] = useState<Win>(() =>
    search.from != null && search.to != null && search.to > search.from
      ? { from: search.from, to: search.to }
      : defaultWindow(),
  );

  const usingCapture = !!captureId;
  const cap = useCaptureReplay(canRead && usingCapture ? captureId : null);
  const window = useWindowReplay(
    canRead && !usingCapture ? win.from : null,
    canRead && !usingCapture ? win.to : null,
  );
  const replay = usingCapture ? cap : window;
  const selectionKey = usingCapture ? captureId : `${win.from}-${win.to}`;

  return (
    <div className="mx-auto flex w-full max-w-6xl flex-col gap-4">
      <Link
        to="/historical"
        className="flex w-fit items-center gap-1 text-sm text-muted-foreground transition-colors hover:text-foreground"
      >
        <ArrowLeft className="size-4" /> Network statistics
      </Link>
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Replay map</h1>
        <p className="text-muted-foreground">
          {replay.data
            ? `${replay.data.flights.length} flights · ${zulu(replay.data.window_start, 0)} – ${zulu(
                replay.data.window_end,
                0,
              )}`
            : "Pick a saved capture or a time window, then play back the recorded traffic on the map."}
        </p>
      </div>

      {!canRead ? (
        <p className="py-16 text-center text-sm text-muted-foreground">
          You don&apos;t have access to network statistics.
        </p>
      ) : (
        <>
          <div className="flex flex-col gap-3 rounded-lg border bg-muted/20 p-4">
            <div className="grid gap-3 md:grid-cols-2">
              <label className="flex flex-col gap-1 text-sm">
                <span className="text-muted-foreground">Capture</span>
                <select
                  className="h-9 rounded-md border bg-background px-2"
                  value={captureId}
                  onChange={(e) => {
                    const v = e.target.value;
                    setCaptureId(v);
                    if (v) patch({ capture: v, from: undefined, to: undefined });
                    else {
                      const w = defaultWindow();
                      setWin(w);
                      patch({ capture: undefined, from: w.from, to: w.to });
                    }
                  }}
                >
                  <option value="">— custom time window —</option>
                  {(captures.data ?? []).map((c) => (
                    <option key={c.id} value={c.id}>
                      {(c.event_title || c.label || "Capture") +
                        ` · ${formatZuluFull(c.start_time)}` +
                        (c.status === "open" ? " (recording)" : "")}
                    </option>
                  ))}
                </select>
              </label>

              {!usingCapture && (
                <div className="flex items-end gap-3">
                  <label className="flex flex-col gap-1 text-xs">
                    <span className="text-muted-foreground">From (Zulu)</span>
                    <input
                      type="datetime-local"
                      className="h-9 rounded-md border bg-background px-2 text-sm"
                      value={toLocalInput(win.from)}
                      onChange={(e) => {
                        const from = fromLocalInput(e.target.value);
                        if (from != null) {
                          setWin((w) => ({ from, to: w.to }));
                          patch({ from, to: win.to, capture: undefined });
                        }
                      }}
                    />
                  </label>
                  <label className="flex flex-col gap-1 text-xs">
                    <span className="text-muted-foreground">To (Zulu)</span>
                    <input
                      type="datetime-local"
                      className="h-9 rounded-md border bg-background px-2 text-sm"
                      value={toLocalInput(win.to)}
                      onChange={(e) => {
                        const to = fromLocalInput(e.target.value);
                        if (to != null) {
                          setWin((w) => ({ from: w.from, to }));
                          patch({ from: win.from, to, capture: undefined });
                        }
                      }}
                    />
                  </label>
                </div>
              )}
            </div>
          </div>

          {replay.isError ? (
            <p className="py-16 text-center text-sm text-muted-foreground">
              That replay isn&apos;t available.
            </p>
          ) : !replay.data ? (
            <p className="py-16 text-center text-sm text-muted-foreground">Loading replay…</p>
          ) : (
            <ReplayMap key={selectionKey} replay={replay.data} />
          )}
        </>
      )}
    </div>
  );
}
