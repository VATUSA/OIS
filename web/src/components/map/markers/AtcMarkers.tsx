import {Marker} from "react-map-gl/maplibre";

import {ATC_COLORS} from "../lib/colors";
import type {AtcData, AtcPositionLite} from "../layers/atc";

const letter = (kind: string) => (kind === "ATIS" ? "A" : kind[0]);

function positionsText(positions: AtcPositionLite[]): string {
  return positions
    .map((p) => {
      const name = p.kind === "ATIS" ? `ATIS${p.atis_code ? " " + p.atis_code : ""}` : p.callsign;
      return `${p.kind} ${name} · ${p.frequency}`;
    })
    .join("\n");
}

/** A staffed airport's stacked DEL/GND/TWR/ATIS pills (native tooltip lists positions + freqs). */
function AtcBadge({ ap }: { ap: AtcData["airports"][number] }) {
  const kinds = ["DEL", "GND", "TWR", "ATIS"].filter((k) => ap.positions.some((p) => p.kind === k));
  if (kinds.length === 0) return null;
  return (
    <Marker longitude={ap.lon} latitude={ap.lat} anchor="center">
      <div style={{ display: "flex", gap: 1 }} title={`${ap.icao}\n${positionsText(ap.positions)}`}>
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
function AreaPill({
  id,
  name,
  color,
  lat,
  lon,
  positions,
}: {
  id: string;
  name?: string | null;
  color: string;
  lat: number;
  lon: number;
  positions: AtcPositionLite[];
}) {
  return (
    <Marker longitude={lon} latitude={lat} anchor="center">
      <span
        title={`${id}${name ? " " + name : ""}\n${positionsText(positions)}`}
        style={{
          display: "inline-block",
          padding: "1px 5px",
          background: "rgba(10,10,10,.85)",
          color,
          font: "700 11px ui-monospace,monospace",
          borderRadius: 4,
          boxShadow: `0 0 0 1px ${color}66`,
          whiteSpace: "nowrap",
        }}
      >
        {id}
      </span>
    </Marker>
  );
}

function ringsCentroid(rings: number[][][]): [number, number] | null {
  const outer = rings[0];
  if (!outer || outer.length === 0) return null;
  let slat = 0;
  let slon = 0;
  for (const [lat, lon] of outer) {
    slat += lat;
    slon += lon;
  }
  return [slat / outer.length, slon / outer.length];
}

/** Centroid [lat, lon] of a boundary feature's outer ring (GeoJSON coords are [lon, lat]). */
function featureCentroid(feat: GeoJSON.Feature): [number, number] | null {
  const geom = feat.geometry;
  const outer =
    geom.type === "Polygon"
      ? geom.coordinates[0]
      : geom.type === "MultiPolygon"
        ? geom.coordinates[0]?.[0]
        : null;
  if (!outer || outer.length === 0) return null;
  let sx = 0;
  let sy = 0;
  for (const [lon, lat] of outer as number[][]) {
    sx += lon;
    sy += lat;
  }
  return [sy / outer.length, sx / outer.length];
}

/** All ATC HTML markers (airport badges + center/TRACON id pills) for the map's ATC layer. */
export function AtcMarkers({
  atc,
  boundaries,
}: {
  atc: AtcData;
  boundaries: GeoJSON.FeatureCollection;
}) {
  const byId = new Map<string, GeoJSON.Feature>();
  for (const f of boundaries.features) {
    const id = String(f.properties?.id ?? "").toUpperCase();
    if (id) byId.set(id, f);
  }
  return (
    <>
      {atc.airports.map((ap) => (
        <AtcBadge key={ap.icao} ap={ap} />
      ))}
      {atc.centers.map((c) => {
        const feat = byId.get(c.id.toUpperCase());
        const at = feat ? featureCentroid(feat) : null;
        if (!at) return null;
        return (
          <AreaPill
            key={c.id}
            id={c.id}
            color={ATC_COLORS.CTR}
            lat={at[0]}
            lon={at[1]}
            positions={c.positions}
          />
        );
      })}
      {atc.tracons.map((t) => {
        const anchor =
          (t.label as [number, number] | null | undefined) ??
          (t.circle as [number, number] | null | undefined) ??
          ringsCentroid(t.rings);
        if (!anchor) return null;
        return (
          <AreaPill
            key={t.id}
            id={t.id}
            name={t.name}
            color={ATC_COLORS.APP}
            lat={anchor[0]}
            lon={anchor[1]}
            positions={t.positions}
          />
        );
      })}
    </>
  );
}
