import {useLayoutEffect, useRef, useState} from "react";
import {Maximize2, Minus, X} from "lucide-react";

import type {AircraftRoute, Fca, FcaFlight} from "@/lib/fca";
import {lineNm, type LatLng} from "@/components/map/lib/geo";
import {useSetting} from "@/lib/settings";
import {hhmmZulu} from "@/lib/time";

/**
 * A draggable/minimizable card showing a clicked aircraft's filed route (and, if an FCA is selected,
 * whether it's in that FCA's crossing sequence). Positioned within the map region and clamped so it
 * can't leave. Ported from the Leaflet map; unchanged behavior.
 */
export function RoutePopup({
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
  const debug = useSetting("debug.enabled", false).value;
  const fixes = route.fixes ?? [];

  const [pos, setPos] = useState({ x: 12, y: 12 });
  const [minimized, setMinimized] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const placed = useRef(false);
  const drag = useRef<{ px: number; py: number; ox: number; oy: number } | null>(null);
  const clampPos = (x: number, y: number) => {
    const el = rootRef.current;
    const parent = el?.offsetParent as HTMLElement | null;
    if (!el || !parent) return { x, y };
    const maxX = Math.max(0, parent.clientWidth - el.offsetWidth);
    const maxY = Math.max(0, parent.clientHeight - el.offsetHeight);
    return { x: Math.min(Math.max(0, x), maxX), y: Math.min(Math.max(0, y), maxY) };
  };
  const onDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    drag.current = { px: e.clientX, py: e.clientY, ox: pos.x, oy: pos.y };
    e.currentTarget.setPointerCapture(e.pointerId);
  };
  const onMove = (e: React.PointerEvent) => {
    if (!drag.current) return;
    setPos(
      clampPos(drag.current.ox + (e.clientX - drag.current.px), drag.current.oy + (e.clientY - drag.current.py)),
    );
  };
  const onUp = () => {
    drag.current = null;
  };
  const stopPointer = (e: React.PointerEvent) => e.stopPropagation();

  // Anchor to the bottom-right corner on first layout (measured, so it works at any map size); after
  // that just re-clamp on resize so a dragged position stays on-screen.
  useLayoutEffect(() => {
    const place = () => {
      const el = rootRef.current;
      const parent = el?.offsetParent as HTMLElement | null;
      if (!el || !parent) return;
      if (!placed.current) {
        placed.current = true;
        setPos({
          x: Math.max(0, parent.clientWidth - el.offsetWidth - 12),
          y: Math.max(0, parent.clientHeight - el.offsetHeight - 12),
        });
      } else {
        setPos((p) => clampPos(p.x, p.y));
      }
    };
    place();
    window.addEventListener("resize", place);
    return () => window.removeEventListener("resize", place);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [minimized]);

  if (minimized) {
    return (
      <div ref={rootRef} className="absolute z-[600]" style={{ left: pos.x, top: pos.y }}>
        <div
          onPointerDown={onDown}
          onPointerMove={onMove}
          onPointerUp={onUp}
          className="flex cursor-move touch-none select-none items-center gap-2 rounded-lg border border-border/70 bg-background/95 px-2.5 py-1.5 shadow-2xl backdrop-blur"
        >
          <span className="font-mono text-sm font-bold text-sky-400">{route.callsign}</span>
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
          <span className="text-lg font-bold tracking-tight text-sky-400">{route.callsign}</span>
          {route.aircraft_type && (
            <span className="ml-2 text-sm text-muted-foreground">{route.aircraft_type}</span>
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
        FAA NASR route{route.nav_cycle ? ` (${route.nav_cycle})` : ""} — fixes, navaids, airways,
        SID/STAR when known.
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

      {debug && fixes.length > 0 && (
        <div className="mt-3 border-t border-border/60 pt-2.5">
          <div className="mb-1 font-mono text-[11px] font-semibold text-amber-500/80">
            DEBUG — per-fix prediction
          </div>
          <div className="max-h-40 overflow-y-auto rounded-lg border border-border/60 bg-muted/20">
            <table className="w-full font-mono text-[11px]">
              <thead className="sticky top-0 bg-muted/60 text-muted-foreground">
                <tr>
                  <th className="px-1.5 py-1 text-left">FIX</th>
                  <th className="px-1.5 py-1 text-right">ETA</th>
                  <th className="px-1.5 py-1 text-right">FL</th>
                  <th className="px-1.5 py-1 text-right">KT</th>
                  <th className="px-1.5 py-1 text-right">HDG</th>
                  <th className="px-1.5 py-1 text-right">NM</th>
                </tr>
              </thead>
              <tbody>
                {fixes.map((f, i) => (
                  <tr key={`${f.name}-${i}`} className="border-t border-border/40">
                    <td className="px-1.5 py-0.5">{f.name}</td>
                    <td className="px-1.5 py-0.5 text-right">{hhmmZulu(f.eta)}</td>
                    <td className="px-1.5 py-0.5 text-right">{Math.round(f.altitude_ft / 100)}</td>
                    <td className="px-1.5 py-0.5 text-right">{f.groundspeed_kt}</td>
                    <td className="px-1.5 py-0.5 text-right">{String(f.heading_deg).padStart(3, "0")}°</td>
                    <td className="px-1.5 py-0.5 text-right">{f.distance_nm}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}

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
            <span className="text-muted-foreground">not crossing this FCA.</span>
          )}
        </div>
      )}
    </div>
  );
}
