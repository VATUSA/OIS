import type {PickingInfo} from "@deck.gl/core";

import {ATC_COLORS} from "./colors";
import type {Theme} from "./constants";
import type {NormAircraft} from "./types";
import {anchorHeader, type AtcAnchor, type AtcPositionLite} from "../layers/atc";

/** VATSIM controller rating id → label. */
const RATINGS: Record<number, string> = {
  2: "S1", 3: "S2", 4: "S3", 5: "C1", 6: "C2", 7: "C3", 8: "I1", 9: "I2", 10: "I3", 11: "SUP", 12: "ADM",
};

const esc = (s: string) =>
  s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c] ?? c);

const posName = (p: AtcPositionLite) =>
  p.kind === "ATIS" ? `ATIS${p.atis_code ? " " + p.atis_code : ""}` : p.callsign;

/** How long the controller has been on position, e.g. "2h14m" (blank if unknown). */
function onlineFor(iso: string): string {
  if (!iso) return "";
  const t = Date.parse(iso);
  if (!Number.isFinite(t)) return "";
  const mins = Math.floor((Date.now() - t) / 60000);
  if (mins < 0) return "";
  const h = Math.floor(mins / 60);
  const m = mins % 60;
  return h > 0 ? `${h}h${m}m` : `${m}m`;
}

/** ATC hover card: header + each position's name, frequency, and controller. */
function atcHtml(a: AtcAnchor, theme: Theme): string {
  const muted = theme === "dark" ? "#94a3b8" : "#64748b";
  const rows = a.positions
    .map((p) => {
      const badge = `<span style="background:${ATC_COLORS[p.kind] ?? "#94a3b8"};color:#0a0a0a;border-radius:3px;padding:0 3px;font:700 10px ui-monospace,monospace">${esc(p.kind)}</span>`;
      const rating = RATINGS[p.rating];
      const online = onlineFor(p.logon_time);
      const ctrl = p.name
        ? `<div style="color:${muted};margin-top:1px">${esc(p.name)}${rating ? ` · ${rating}` : ""}${online ? ` · ${online}` : ""}</div>`
        : "";
      return `<div style="margin-top:4px"><span style="font-family:ui-monospace,monospace">${badge} <span style="font-weight:600">${esc(posName(p))}</span> <span style="color:${muted}">${esc(p.frequency)}</span></span>${ctrl}</div>`;
    })
    .join("");
  return `<div style="font:700 13px ui-monospace,monospace">${esc(anchorHeader(a))}</div>${rows}`;
}

/** Hover cards for the map: aircraft glyphs, matched traffic, and ATC labels (position + controller). */
export function mapTooltip(theme: Theme) {
  const style = {
    background: theme === "dark" ? "#111418" : "#ffffff",
    color: theme === "dark" ? "#e6edf3" : "#1b1f24",
    fontSize: "12px",
    padding: "6px 8px",
    borderRadius: "6px",
    boxShadow: "0 2px 8px rgba(0,0,0,.3)",
    maxWidth: "260px",
  };
  return (info: PickingInfo) => {
    const id = info.layer?.id;
    if (id === "aircraft" || id === "matched") {
      const d = info.object as NormAircraft | undefined;
      if (!d) return null;
      return {
        html:
          `<div style="font-weight:600">${esc(d.callsign)}</div>` +
          `<div>${esc(d.dep || "????")} → ${esc(d.arr || "????")}</div>` +
          `<div>${esc(d.actype || "—")} · ${d.alt}ft · ${d.gs}kt</div>`,
        style,
      };
    }
    if (id === "atc-hover") {
      const a = info.object as AtcAnchor | undefined;
      if (!a) return null;
      return { html: atcHtml(a, theme), style };
    }
    return null;
  };
}
