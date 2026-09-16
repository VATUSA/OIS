import {useLayoutEffect, useRef, useState} from "react";
import {type DataColumn, DataTable} from "@ois/ui";
import {Maximize2, Minus, X} from "lucide-react";

import type {AircraftRoute, Fca, FcaFlight} from "@/lib/fca";
import {lineNm, type LatLng} from "@/components/map/lib/geo";
import {useSetting} from "@/lib/settings";
import {hhmmZulu} from "@/lib/time";

type Fix = NonNullable<AircraftRoute["fixes"]>[number];

const FIX_COLUMNS: DataColumn<Fix>[] = [
  { accessorKey: "name", header: "Fix", mono: true },
  { accessorKey: "eta", header: "ETA", mono: true, align: "right", cell: (c) => hhmmZulu(c.row.original.eta) },
  {
    accessorKey: "altitude_ft",
    header: "FL",
    mono: true,
    align: "right",
    cell: (c) => Math.round(c.row.original.altitude_ft / 100),
  },
  { accessorKey: "groundspeed_kt", header: "KT", mono: true, align: "right" },
  {
    accessorKey: "heading_deg",
    header: "HDG",
    mono: true,
    align: "right",
    cell: (c) => `${String(c.row.original.heading_deg).padStart(3, "0")}°`,
  },
  { accessorKey: "distance_nm", header: "NM", mono: true, align: "right" },
];

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
          className="flex cursor-move touch-none select-none items-center gap-2 rounded-full border border-line bg-panel py-1 pl-3 pr-1.5"
        >
          <span className="font-mono text-sm font-bold text-map-highlight">{route.callsign}</span>
          <button
            type="button"
            onPointerDown={stopPointer}
            onClick={() => setMinimized(false)}
            className="rounded-full p-1 text-ink-3 transition-colors hover:bg-panel-2 hover:text-ink"
            aria-label="Expand"
          >
            <Maximize2 className="size-3.5" />
          </button>
          <button
            type="button"
            onPointerDown={stopPointer}
            onClick={onClose}
            className="rounded-full p-1 text-ink-3 transition-colors hover:bg-panel-2 hover:text-ink"
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
      className="absolute z-[600] w-[min(92vw,26rem)] rounded-md border border-line bg-panel p-4"
      style={{ left: pos.x, top: pos.y }}
    >
      <div
        onPointerDown={onDown}
        onPointerMove={onMove}
        onPointerUp={onUp}
        className="flex cursor-move touch-none select-none items-start justify-between gap-3"
      >
        <div className="min-w-0 font-mono">
          <span className="text-lg font-bold tracking-tight text-map-highlight">{route.callsign}</span>
          {route.aircraft_type && (
            <span className="ml-2 text-sm text-ink-2">{route.aircraft_type}</span>
          )}
        </div>
        <div className="-mr-1 -mt-1 flex items-center gap-0.5">
          <button
            type="button"
            onPointerDown={stopPointer}
            onClick={() => setMinimized(true)}
            className="rounded-full p-1 text-ink-3 transition-colors hover:bg-panel-2 hover:text-ink"
            aria-label="Minimize"
          >
            <Minus className="size-4" />
          </button>
          <button
            type="button"
            onPointerDown={stopPointer}
            onClick={onClose}
            className="rounded-full p-1 text-ink-3 transition-colors hover:bg-panel-2 hover:text-ink"
            aria-label="Close"
          >
            <X className="size-4" />
          </button>
        </div>
      </div>

      <div className="mt-1 font-mono text-sm text-ink-2">
        {route.dep || "????"} → {route.arr || "????"}
        {route.altitude > 0 && <> · FL{Math.round(route.altitude / 100)}</>}
        {route.groundspeed > 0 && <> · {route.groundspeed}kt</>}
        {nm > 0 && <> · {nm} NM</>} · {pts.length} pts
      </div>

      <div className="mt-3 max-h-40 overflow-y-auto rounded-xs border border-line bg-panel-2 p-2.5 font-mono text-xs leading-relaxed break-words">
        {route.route || "(no filed route)"}
      </div>

      <p className="mt-2 text-[11px] leading-snug text-ink-3">
        FAA NASR route{route.nav_cycle ? ` (${route.nav_cycle})` : ""} — fixes, navaids, airways,
        SID/STAR when known.
        {unresolved.length > 0 && (
          <>
            {" "}
            <span className="text-warning">
              Unresolved: {unresolved.slice(0, 14).join(", ")}
              {unresolved.length > 14 ? "…" : ""}.
            </span>
          </>
        )}
      </p>

      {debug && fixes.length > 0 && (
        <div className="mt-3 border-t border-line pt-2.5">
          <div className="mb-1 font-mono text-[11px] font-semibold text-warning">
            DEBUG — per-fix prediction
          </div>
          <div className="max-h-40 overflow-y-auto">
            <DataTable
              columns={FIX_COLUMNS}
              data={fixes}
              getRowId={(f, i) => `${f.name}-${i}`}
              rowCap={fixes.length}
              stickyHeader
              label="Per-fix prediction"
              className="[&_table]:text-[11px] [&_td]:px-1.5 [&_td]:py-0.5 [&_th]:px-1.5 [&_th]:py-1"
            />
          </div>
        </div>
      )}

      {fca && (
        <div className="mt-3 flex items-baseline gap-1.5 border-t border-line pt-2.5 font-mono text-xs">
          <span className="font-semibold" style={{ color: fca.color }}>
            {fca.name}
          </span>
          {match ? (
            <span className="text-success">
              IN SEQUENCE #{match.seq} — crosses in {match.distance_nm}nm.
            </span>
          ) : (
            <span className="text-ink-2">not crossing this FCA.</span>
          )}
        </div>
      )}
    </div>
  );
}
