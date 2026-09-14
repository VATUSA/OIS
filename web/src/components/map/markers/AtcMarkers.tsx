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
      <div style={{ display: "flex", gap: 1, pointerEvents: "none" }}>
        {kinds.map((k) => (
          <span
            key={k}
            style={{
              display: "inline-flex",
              alignItems: "center",
              justifyContent: "center",
              width: 14,
              height: 14,
              background: ATC_COLORS[k] ?? "#94a3b8",
              color: "#0a0a0a",
              font: "800 10px ui-monospace,monospace",
              borderRadius: 3,
              boxShadow: "0 0 0 1px rgba(0,0,0,.5)",
            }}
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
        style={{
          display: "inline-block",
          padding: "1px 5px",
          background: "rgba(10,10,10,.85)",
          color,
          font: "700 11px ui-monospace,monospace",
          borderRadius: 4,
          boxShadow: `0 0 0 1px ${color}66`,
          whiteSpace: "nowrap",
          pointerEvents: "none",
        }}
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
