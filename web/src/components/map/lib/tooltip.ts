import type {PickingInfo} from "@deck.gl/core";

import {RATINGS, onlineFor} from "@/lib/atc-format";
import {ATC_COLORS} from "./colors";
import type {Theme} from "./constants";
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

/** Hover cards for the map: aircraft glyphs, matched traffic, and ATC labels (position + controller). */
export function mapTooltip(_theme?: Theme) {
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
    if (id === "aircraft" || id === "matched") {
      // The plain "aircraft" layer holds NormAircraft (actype/alt/gs); the "matched" (in-FCA) layer
      // holds MatchedFlight (aircraft_type/altitude/groundspeed). Read whichever the object carries.
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
          `<div>${esc(actype || "—")} · ${num(alt, "ft")} · ${num(gs, "kt")}</div>`,
        style,
      };
    }
    if (id === "atc-hover") {
      const a = info.object as AtcAnchor | undefined;
      if (!a) return null;
      return { html: atcHtml(a), style };
    }
    return null;
  };
}
