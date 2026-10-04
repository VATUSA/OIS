import type {PickingInfo} from "@deck.gl/core";

import {RATINGS, onlineFor} from "@/lib/atc-format";
import {DELAY_THRESHOLD_SEC, fmtDelaySec} from "@/lib/fca";
import {hhmmZulu} from "@/lib/time";
import {ATC_COLORS} from "./colors";
import {objectUnder} from "./pick";
import type {NormAircraft} from "./types";
import type {MatchedFlight} from "../layers/matched";
import {anchorHeader, type AtcAnchor, type AtcPositionLite} from "../layers/atc";
import {SECTOR_LAYER_ID, SECTOR_TIERS} from "../layers/sectors";
import type {SectorVolume} from "@/lib/sectors";

/** The hover card's look, shared by every map card so they read as one design (tokens only, no shadow). */
const CARD_STYLE = {
  background: "var(--panel)",
  color: "var(--ink)",
  border: "1px solid var(--line)",
  fontSize: "12px",
  padding: "6px 8px",
  borderRadius: "6px",
  boxShadow: "none",
  maxWidth: "260px",
};

const esc = (s: string) =>
  s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c] ?? c);

const posName = (p: AtcPositionLite) =>
  p.kind === "ATIS" ? `ATIS${p.atis_code ? " " + p.atis_code : ""}` : p.callsign;

/** ATC hover card: header + each position's name, frequency, and controller. */
function atcHtml(a: AtcAnchor): string {
  const muted = "var(--ink-2)";
  const rows = a.positions
    .map((p) => {
      const badge = `<span style="background:${ATC_COLORS[p.kind] ?? "var(--ink-3)"};color:var(--ground);border-radius:6px;padding:0 3px;font:700 10px 'JetBrains Mono',ui-monospace,monospace">${esc(p.kind)}</span>`;
      const rating = RATINGS[p.rating];
      const online = onlineFor(p.logon_time);
      const ctrl = p.name
        ? `<div style="color:${muted};margin-top:1px">${esc(p.name)}${rating ? ` · ${rating}` : ""}${online ? ` · ${online}` : ""}</div>`
        : "";
      return `<div style="margin-top:4px"><span style="font-family:'JetBrains Mono',ui-monospace,monospace">${badge} <span style="font-weight:600">${esc(posName(p))}</span> <span style="color:${muted}">${esc(p.frequency)}</span></span>${ctrl}</div>`;
    })
    .join("");
  return `<div style="font:700 13px 'JetBrains Mono',ui-monospace,monospace">${esc(anchorHeader(a))}</div>${rows}`;
}

/** Metering line for an in-FCA aircraft: sequence, STA, ETA, and delay (same threshold as the FCA detail page). */
function meteringHtml(f: MatchedFlight): string {
  const delayed = f.delay_sec >= DELAY_THRESHOLD_SEC;
  const delay = delayed
    ? `<span style="color:var(--danger)">+${fmtDelaySec(f.delay_sec)}</span>`
    : `<span style="color:var(--success)">on time</span>`;
  return `<div style="font-family:'JetBrains Mono',ui-monospace,monospace">#${f.seq} · STA ${hhmmZulu(f.cross_time)} · ETA ${hhmmZulu(f.eta)} · ${delay}</div>`;
}

/**
 * Whether the aircraft is the nearer of two overlapping hover targets to the cursor (#555), comparing
 * their projected centres in screen pixels. A tie goes to the pill.
 */
export function nearerIsAircraft(
  cursor: [number, number],
  aircraft: [number, number],
  pill: [number, number],
): boolean {
  const dist = ([x, y]: [number, number]) => Math.hypot(x - cursor[0], y - cursor[1]);
  return dist(aircraft) < dist(pill);
}

type Projector = { project: (lngLat: [number, number]) => number[] };

/** `[lon, lat]` → screen pixels through the pick's viewport, or `null` without one (as in unit tests). */
function toScreen(info: PickingInfo, lon: number, lat: number): [number, number] | null {
  const viewport = (info as { viewport?: Projector }).viewport;
  if (!viewport) return null;
  const [x, y] = viewport.project([lon, lat]);
  return [x, y];
}

/** An ATC hover card for `a`, or no card at all when there's no anchor there. */
function atcCard(a: AtcAnchor | null | undefined, style: Record<string, string>) {
  return a ? { html: atcHtml(a), style } : null;
}

/**
 * The hover-card renderer for the current settings, or `undefined` when tooltips are off entirely
 * (deck then draws no card). `map.aircraftTooltips` only narrows what `map.tooltips` allows, and
 * with tooltips off the map also drops the invisible ATC hover targets (see `TrafficMap`).
 */
export function tooltipFor(settings: { tooltips: boolean; aircraft: boolean }) {
  if (!settings.tooltips) return undefined;
  return settings.aircraft ? ALL_TOOLTIP : ATC_ONLY_TOOLTIP;
}

/**
 * Hover cards for the map: aircraft glyphs, matched (in-FCA) traffic, and ATC labels (position +
 * controller). `aircraft: false` drops the aircraft and matched cards, leaving ATC.
 */
export function mapTooltip({ aircraft = true }: { aircraft?: boolean } = {}) {
  const style = {
    background: "var(--panel)",
    color: "var(--ink)",
    border: "1px solid var(--line)",
    fontSize: "12px",
    padding: "6px 8px",
    borderRadius: "6px",
    boxShadow: "none",
    maxWidth: "260px",
  };
  return (info: PickingInfo) => {
    const id = info.layer?.id;
    // Matched glyph layers are "matched" (one FCA) or "matched-<fcaId>" (overview); their sibling
    // trail/dot/badge layers aren't pickable, so any "matched" pick is a glyph.
    if (id === "aircraft" || id?.startsWith("matched")) {
      // Look past the glyph for an ATC pill underneath, whatever the aircraft setting. A plane
      // parked on a staffed airport's badge wins the pick — the aircraft IconLayer is a 48x48 masked
      // icon sitting above `atc-hover`, against a 13-19px ATC circle — and at exactly the airports
      // that draw a DEL/GND/TWR/ATIS stack there is usually a plane on the badge. #323 added this
      // re-pick but applied it only when aircraft cards were off, so with default settings the ATC
      // card could never render while aircraft tooltips visibly worked (#477).
      const anchor = objectUnder(info, "atc-hover") as AtcAnchor | null;
      // The plain "aircraft" layer holds NormAircraft (actype/alt/gs); the matched (in-FCA) layers
      // hold MatchedFlight (aircraft_type/altitude/groundspeed + metering). Read whichever it carries.
      const d = info.object as (NormAircraft & Partial<MatchedFlight>) | undefined;
      // When both overlap, the nearer target wins (#555). #477 gave the pill every overlap, which made
      // the aircraft card unreachable at the staffed airports operators watch most. Comparing the
      // cursor's distance to each centre keeps both reachable — nudge toward the one you want — and a
      // tie (or no viewport to measure with) still goes to the pill, the smaller and more deliberate
      // target. With aircraft cards off there is nothing to compare: the pill shows.
      if (anchor) {
        const plane = aircraft && d ? toScreen(info, d.lon, d.lat) : null;
        const badge = toScreen(info, anchor.lon, anchor.lat);
        if (!plane || !badge || !nearerIsAircraft([info.x, info.y], plane, badge)) {
          return atcCard(anchor, style);
        }
      }
      // With aircraft cards off and nothing underneath, no card rather than an empty one.
      if (!aircraft) return null;
      if (!d) return null;
      const actype = d.actype || d.aircraft_type || "";
      const alt = d.alt ?? d.altitude;
      const gs = d.gs ?? d.groundspeed;
      const num = (v: number | undefined, unit: string) =>
        v == null ? "—" : `${v}${unit}`;
      return {
        html:
          `<div style="font-weight:600">${esc(d.callsign)}</div>` +
          `<div>${esc(d.dep || "????")} → ${esc(d.arr || "????")}</div>` +
          `<div>${esc(actype || "—")} · ${num(alt, "ft")} · ${num(gs, "kt")}</div>` +
          (d.seq != null ? meteringHtml(d as MatchedFlight) : ""),
        style,
      };
    }
    if (id === "atc-hover") return atcCard(info.object as AtcAnchor | undefined, style);
    return null;
  };
}

const ALL_TOOLTIP = mapTooltip();
const ATC_ONLY_TOOLTIP = mapTooltip({ aircraft: false });

/**
 * Hover card for a parking stand on the airport surface map (#517).
 *
 * #431 took gate coverage from one airport to 183, and most of what it imported is not a terminal
 * gate: 7,129 of 12,962 stands are GA tie-downs. The kind is stored and served but was displayed
 * nowhere, so an operator editing surface data at a GA-heavy field saw a mass of identical dots. This
 * is the "what am I looking at" half; `buildSurfaceLayers` draws non-gate stands smaller for the
 * at-a-glance half.
 *
 * Shares `mapTooltip`'s card shape and token-only style on purpose — the surface card should look like
 * the traffic and ATC cards an operator already reads, not like a second design.
 */
export function surfaceTooltip() {
  return (info: PickingInfo) => {
    if (info.layer?.id !== "surface-gates") return null;
    const d = info.object as { name?: string; kind?: string | null; source?: string } | undefined;
    if (!d?.name) return null;
    return { html: standHtml(d), style: CARD_STYLE };
  };
}

/** The admin sector map's hover card (#602): sector, tier and vertical band, in the same card shape. */
export function sectorTooltip() {
  return (info: PickingInfo) => {
    if (info.layer?.id !== SECTOR_LAYER_ID) return null;
    const v = (info.object as { volume?: SectorVolume } | undefined)?.volume;
    return v ? { html: sectorHtml(v), style: CARD_STYLE } : null;
  };
}

/** `SFC` for a floor at the surface, else a flight level (`FL240`). */
export const flightLevel = (ft: number) => (ft <= 0 ? "SFC" : `FL${String(Math.round(ft / 100)).padStart(3, "0")}`);

/**
 * The sector card: `ZDC 32 · High`, its name, and `FL240–FL350`. Every field goes through `esc` — the
 * name comes from imported data and this is rendered as `html`, so an unescaped name would be a
 * stored-XSS vector.
 */
export function sectorHtml(v: SectorVolume): string {
  const mono = "'JetBrains Mono',ui-monospace,monospace";
  const tier = SECTOR_TIERS.find((t) => t.tier === v.tier)?.label ?? v.tier;
  const name = v.name ? `<div>${esc(v.name)}</div>` : "";
  const band = `${flightLevel(v.base_alt_ft)}–${flightLevel(v.top_alt_ft)}`;
  return (
    `<div style="font:700 13px ${mono}">${esc(v.artcc)} ${esc(v.sector_id)} · ${esc(tier)}</div>${name}` +
    `<div style="color:var(--ink-2);font-family:${mono}">${esc(band)}</div>`
  );
}

/**
 * The stand card's markup: name, kind badge, source.
 *
 * Everything interpolated goes through `esc`. A stand name is both operator-editable and imported from
 * a community-contributed X-Plane extract, and this is rendered as `html` — so an unescaped name would
 * be a stored-XSS vector, not merely a display bug.
 *
 * An absent `kind` drops the badge entirely rather than rendering an empty one. Every `manual`, `osm`
 * and `crc` row has no kind, and labelling them would assert a stand type nothing ever imported.
 */
function standHtml(d: { name?: string; kind?: string | null; source?: string }): string {
  const muted = "var(--ink-2)";
  const mono = "'JetBrains Mono',ui-monospace,monospace";
  // `var(--ink-3)` rather than a second hue: DESIGN.md allows one accent, and the kind is a
  // classification rather than a status, so it should not compete with the accent for attention.
  const badge = d.kind
    ? `<span style="background:var(--ink-3);color:var(--ground);border-radius:6px;padding:0 3px;font:700 10px ${mono}">${esc(d.kind)}</span> `
    : "";
  const source = d.source
    ? `<div style="color:${muted};margin-top:1px">${esc(d.source)}</div>`
    : "";
  return `<div style="font:700 13px ${mono}">${badge}${esc(d.name ?? "")}</div>${source}`;
}
