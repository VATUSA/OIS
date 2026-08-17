import {useEffect, useMemo, useRef, useState} from "react";
import L from "leaflet";
import "leaflet/dist/leaflet.css";
import {Button, useTheme} from "@ois/ui";
import {Link, useParams} from "@tanstack/react-router";
import {ArrowLeft, Pause, Play, SkipBack} from "lucide-react";

import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {type Replay, useCaptureReplay} from "@/lib/stats";
import {
  type AircraftCanvasLayer,
  aircraftCanvasLayer,
  type CanvasAircraft,
} from "@/components/aircraft-canvas-layer";

const CARTO = {
  dark: "https://{s}.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}{r}.png",
  light: "https://{s}.basemaps.cartocdn.com/light_all/{z}/{x}/{y}{r}.png",
} as const;
const MAP_BG = { dark: "#0a0a0a", light: "#e5e7eb" } as const;
const US_HOME = { center: [39.5, -98.35] as [number, number], zoom: 4.3 };

const SPEEDS = [1, 2, 4, 8, 16, 32, 64];
const EMPTY = new Set<string>();

/** A flight's samples as a flat, sorted array of [t, lat, lon, alt, hdg]. */
type Track = {
  callsign: string;
  dep: string;
  arr: string;
  actype: string;
  s: number[][];
};

function zulu(base: string, offsetS: number): string {
  const d = new Date(Date.parse(base) + offsetS * 1000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getUTCDate())}/${p(d.getUTCHours())}${p(d.getUTCMinutes())}${p(d.getUTCSeconds())}z`;
}

function ReplayMap({ replay }: { replay: Replay }) {
  const { resolvedTheme } = useTheme();
  const nodeRef = useRef<HTMLDivElement>(null);
  const mapRef = useRef<L.Map | null>(null);
  const tileRef = useRef<L.TileLayer | null>(null);
  const layerRef = useRef<AircraftCanvasLayer | null>(null);

  const duration = useMemo(
    () => Math.max(1, (Date.parse(replay.window_end) - Date.parse(replay.window_start)) / 1000),
    [replay],
  );

  const tracks = useMemo<Track[]>(
    () =>
      replay.flights.map((f) => ({
        callsign: f.callsign,
        dep: f.departure ?? "",
        arr: f.arrival ?? "",
        actype: f.aircraft ?? "",
        s: f.samples as number[][],
      })),
    [replay],
  );

  // Refs the animation loop reads (avoids stale closures); UI state mirrors them.
  const clockRef = useRef(0);
  const playingRef = useRef(false);
  const speedRef = useRef(1);
  const [clock, setClock] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [speed, setSpeed] = useState(1);
  const [active, setActive] = useState(0);

  // Render the aircraft set at time `t` (seconds from window start).
  function renderAt(t: number) {
    const layer = layerRef.current;
    if (!layer) return;
    const list: CanvasAircraft[] = [];
    for (const f of tracks) {
      const s = f.s;
      if (s.length === 0 || t < s[0][0] || t > s[s.length - 1][0]) continue;
      // Binary search for the segment containing `t`.
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
      list.push({
        callsign: f.callsign,
        lat: a[1] + (b[1] - a[1]) * k,
        lon: a[2] + (b[2] - a[2]) * k,
        alt: Math.round(a[3] + (b[3] - a[3]) * k),
        heading: a[4],
        actype: f.actype,
        dep: f.dep,
        arr: f.arr,
        gs: 0,
      });
    }
    layer.setData(list, EMPTY, true);
    setActive(list.length);
  }

  // Map init (once).
  useEffect(() => {
    const node = nodeRef.current;
    if (!node || mapRef.current) return;
    const map = L.map(node, { zoomControl: false, zoomSnap: 0, preferCanvas: true }).setView(
      US_HOME.center,
      US_HOME.zoom,
    );
    L.control.zoom({ position: "topright" }).addTo(map);
    node.style.background = MAP_BG[resolvedTheme];
    tileRef.current = L.tileLayer(CARTO[resolvedTheme], {
      maxZoom: 14,
      attribution: "© OpenStreetMap, © CARTO · traffic: VATSIM",
    }).addTo(map);
    const layer = aircraftCanvasLayer({}) as AircraftCanvasLayer;
    layer.addTo(map);
    layerRef.current = layer;
    mapRef.current = map;
    renderAt(0);
    return () => {
      map.remove();
      mapRef.current = null;
      layerRef.current = null;
      tileRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Re-render aircraft when the track set changes.
  useEffect(() => {
    clockRef.current = 0;
    setClock(0);
    renderAt(0);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tracks]);

  // Theme swap.
  useEffect(() => {
    tileRef.current?.setUrl(CARTO[resolvedTheme]);
    if (nodeRef.current) nodeRef.current.style.background = MAP_BG[resolvedTheme];
  }, [resolvedTheme]);

  // Animation loop: advance the clock while playing; re-render at ~15 fps.
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
      if (now - lastRender >= 66) {
        lastRender = now;
        renderAt(clockRef.current);
        setClock(clockRef.current);
      }
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(raf);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [duration]);

  const toggle = () => {
    const next = !playingRef.current;
    // Restart from the beginning if we're parked at the end.
    if (next && clockRef.current >= duration) {
      clockRef.current = 0;
      setClock(0);
    }
    playingRef.current = next;
    setPlaying(next);
  };

  const scrub = (v: number) => {
    clockRef.current = v;
    setClock(v);
    renderAt(v);
  };

  const setSpd = (v: number) => {
    speedRef.current = v;
    setSpeed(v);
  };

  return (
    <div className="flex flex-col gap-3">
      <div className="relative h-[70vh] w-full overflow-hidden rounded-lg border">
        <div ref={nodeRef} className="absolute inset-0" />
        <div className="pointer-events-none absolute left-3 top-3 z-[500] rounded-md bg-background/80 px-3 py-1.5 text-sm shadow backdrop-blur">
          <span className="font-mono font-medium">{zulu(replay.window_start, clock)}</span>
          <span className="ml-2 text-muted-foreground">{active} aircraft</span>
        </div>
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
          className="h-2 flex-1 min-w-[200px] cursor-pointer accent-primary"
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

export function CaptureReplayPage() {
  const { captureId } = useParams({ from: "/stats/captures/$captureId/replay" });
  const { data: me } = useMe();
  const canRead = hasPermission(me, "stats.data.read");
  const replay = useCaptureReplay(canRead ? captureId : null);

  return (
    <div className="mx-auto flex w-full max-w-6xl flex-col gap-4">
      <Link
        to="/stats"
        className="flex w-fit items-center gap-1 text-sm text-muted-foreground transition-colors hover:text-foreground"
      >
        <ArrowLeft className="size-4" /> Network statistics
      </Link>
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Capture replay</h1>
        <p className="text-muted-foreground">
          {replay.data
            ? `${replay.data.flights.length} flights · ${zulu(
                replay.data.window_start,
                0,
              )} – ${zulu(replay.data.window_end, 0)}`
            : "Play back the recorded traffic on the map."}
        </p>
      </div>

      {!canRead ? (
        <p className="py-16 text-center text-sm text-muted-foreground">
          You don&apos;t have access to network statistics.
        </p>
      ) : replay.isError ? (
        <p className="py-16 text-center text-sm text-muted-foreground">
          That capture isn&apos;t available.
        </p>
      ) : !replay.data ? (
        <p className="py-16 text-center text-sm text-muted-foreground">Loading replay…</p>
      ) : (
        <ReplayMap replay={replay.data} />
      )}
    </div>
  );
}
