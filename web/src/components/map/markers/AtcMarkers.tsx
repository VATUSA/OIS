import {Marker} from "react-map-gl/maplibre";

import {ATC_COLORS} from "../lib/colors";
import {atcBadgeKinds, type AtcAnchor, type AtcPositionLite} from "../layers/atc";

const letter = (kind: string) => (kind === "ATIS" ? "A" : kind[0]);

/** A staffed airport's stacked DEL/GND/TWR/ATIS pills. */
function AtcBadge({ lat, lon, positions }: { lat: number; lon: number; positions: AtcPositionLite[] }) {
  const kinds = atcBadgeKinds(positions);
  if (kinds.length === 0) return null;
  return (
    <Marker longitude={lon} latitude={lat} anchor="center">
      <div className="pointer-events-none flex gap-px">
        {kinds.map((k) => (
          <span
            key={k}
            className="inline-flex size-3.5 items-center justify-center rounded-xs font-mono text-[10px] font-bold leading-none text-ground outline outline-1 outline-ground/50"
            style={{ background: ATC_COLORS[k] ?? "var(--ink-3)" }}
          >
            {letter(k)}
          </span>
        ))}
      </div>
    </Marker>
  );
}

/** A TRACON/center id pill anchored on its area. */
function AreaPill({ id, color, lat, lon }: { id: string; color: string; lat: number; lon: number }) {
  return (
    <Marker longitude={lon} latitude={lat} anchor="center">
      <span
        className="pointer-events-none inline-block whitespace-nowrap rounded-xs bg-panel px-[5px] py-px font-mono text-[11px] font-bold"
        style={{ color, outline: `1px solid color-mix(in srgb, ${color} 40%, transparent)` }}
      >
        {id}
      </span>
    </Marker>
  );
}

/**
 * The ATC label pills (airport badges + center/TRACON id pills). Display only — hovering is handled by
 * an invisible pickable deck layer (`atc-hover`) + getTooltip, because these DOM markers sit under
 * deck's event layer and can't receive hover directly.
 */
export function AtcMarkers({ anchors }: { anchors: AtcAnchor[] }) {
  return (
    <>
      {anchors.map((a, i) =>
        a.type === "airport" ? (
          <AtcBadge key={`ap-${a.icao}-${i}`} lat={a.lat} lon={a.lon} positions={a.positions} />
        ) : (
          <AreaPill key={`ar-${a.id}-${i}`} id={a.id} color={a.color} lat={a.lat} lon={a.lon} />
        ),
      )}
    </>
  );
}
