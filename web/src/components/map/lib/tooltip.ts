import type {PickingInfo} from "@deck.gl/core";

import {RATINGS, onlineFor} from "@/lib/atc-format";
import {DELAY_THRESHOLD_SEC, fmtDelaySec} from "@/lib/fca";
import {hhmmZulu} from "@/lib/time";
import {ATC_COLORS} from "./colors";
import {objectUnder} from "./pick";
import type {NormAircraft} from "./types";
import type {MatchedFlight} from "../layers/matched";
import {anchorHeader, type AtcAnchor, type AtcPositionLite} from "../layers/atc";

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
      // With aircraft cards off, look past the glyph for an ATC pill underneath: a plane parked on
      // a staffed airport's badge wins the pick, and returning null here would blank the pill's
      // card too, even though only *aircraft* tooltips were turned off (#323).
      if (!aircraft) return atcCard(objectUnder(info, "atc-hover") as AtcAnchor | null, style);
      // The plain "aircraft" layer holds NormAircraft (actype/alt/gs); the matched (in-FCA) layers
      // hold MatchedFlight (aircraft_type/altitude/groundspeed + metering). Read whichever it carries.
      const d = info.object as (NormAircraft & Partial<MatchedFlight>) | undefined;
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
