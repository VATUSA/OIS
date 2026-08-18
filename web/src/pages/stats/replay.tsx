import {useEffect, useMemo, useRef, useState} from "react";
import DeckGL from "@deck.gl/react";
import {IconLayer} from "@deck.gl/layers";
import {Map as MapLibre} from "react-map-gl/maplibre";
import "maplibre-gl/dist/maplibre-gl.css";
import {Button, useTheme} from "@ois/ui";
import {Link, useParams} from "@tanstack/react-router";
import {ArrowLeft, Pause, Play, SkipBack} from "lucide-react";

import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {type Replay, useCaptureReplay} from "@/lib/stats";
import {aircraftIconUrl} from "@/lib/aircraft-icons";

// Free CARTO vector basemap styles (no access token needed).
const CARTO_STYLE = {
  dark: "https://basemaps.cartocdn.com/gl/dark-matter-gl-style/style.json",
  light: "https://basemaps.cartocdn.com/gl/positron-gl-style/style.json",
} as const;
const INITIAL_VIEW = { longitude: -98.35, latitude: 39.5, zoom: 3.4 };
const SPEEDS = [1, 2, 4, 8, 16, 32, 64];

/** One aircraft rendered at the current replay clock. */
type Live = {
  callsign: string;
  actype: string;
  lon: number;
  lat: number;
  alt: number;
  heading: number;
};

/** A flight's samples as a sorted array of [t, lat, lon, alt, hdg]. */
type Track = { callsign: string; actype: string; s: number[][] };

function zulu(base: string, offsetS: number): string {
  const d = new Date(Date.parse(base) + offsetS * 1000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getUTCDate())}/${p(d.getUTCHours())}${p(d.getUTCMinutes())}${p(d.getUTCSeconds())}z`;
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
        callsign: f.callsign,
        actype: f.aircraft ?? "",
        s: f.samples as number[][],
      })),
    [replay],
  );

  // The animation loop reads refs; UI state mirrors them.
  const clockRef = useRef(0);
  const playingRef = useRef(false);
  const speedRef = useRef(1);
  const [clock, setClock] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [speed, setSpeed] = useState(1);
  const [aircraft, setAircraft] = useState<Live[]>([]);

  // Interpolate every flight's position at time `t` (seconds from window start).
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
        callsign: f.callsign,
        actype: f.actype,
        lat: a[1] + (b[1] - a[1]) * k,
        lon: a[2] + (b[2] - a[2]) * k,
        alt: Math.round(a[3] + (b[3] - a[3]) * k),
        heading: a[4],
      });
    }
    return out;
  }

  const render = (t: number) => {
    setAircraft(frameAt(t));
    setClock(t);
  };

  // Reset to the start when a new capture loads.
  useEffect(() => {
    clockRef.current = 0;
    render(0);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tracks]);

  // Nudge deck.gl to re-measure once layout has settled — its container can be 0-sized at first
  // paint (route transition / off-screen mount), which otherwise leaves the canvas blank.
  useEffect(() => {
    const t = setTimeout(() => window.dispatchEvent(new Event("resize")), 150);
    return () => clearTimeout(t);
  }, []);

  // rAF loop: advance the clock while playing; re-render at ~30 fps (GPU draws the icons).
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

  const iconColor: [number, number, number] = resolvedTheme === "dark" ? [255, 190, 70] : [40, 60, 90];

  const layers = [
    new IconLayer<Live>({
      id: "aircraft",
      data: aircraft,
      getIcon: (d) => {
        const url = aircraftIconUrl(d.actype);
        return { id: url, url, width: 48, height: 48, mask: true };
      },
      getPosition: (d) => [d.lon, d.lat],
      getAngle: (d) => 360 - d.heading,
      getColor: iconColor,
      getSize: 26,
      sizeUnits: "pixels",
      billboard: false,
      // The whole set is replaced each tick; recolor on theme change.
      updateTriggers: { getColor: [resolvedTheme] },
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
          style={{ position: "absolute", top: "0", left: "0", width: "100%", height: "100%" }}
        >
          <MapLibre mapStyle={CARTO_STYLE[resolvedTheme]} attributionControl={false} />
        </DeckGL>
        <div className="pointer-events-none absolute left-3 top-3 z-10 rounded-md bg-background/80 px-3 py-1.5 text-sm shadow backdrop-blur">
          <span className="font-mono font-medium">{zulu(replay.window_start, clock)}</span>
          <span className="ml-2 text-muted-foreground">{aircraft.length} aircraft</span>
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
            ? `${replay.data.flights.length} flights · ${zulu(replay.data.window_start, 0)} – ${zulu(
                replay.data.window_end,
                0,
              )}`
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
