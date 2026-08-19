import type {PickingInfo} from "@deck.gl/core";

import type {Theme} from "./constants";
import type {NormAircraft} from "./types";

/** Hover card for an aircraft glyph (callsign, dep→arr, type·alt·gs). */
export function aircraftTooltip(theme: Theme) {
  return (info: PickingInfo) => {
    const d = info.object as NormAircraft | undefined;
    if (!d || (info.layer?.id !== "aircraft" && info.layer?.id !== "matched")) return null;
    return {
      html:
        `<div style="font-weight:600">${d.callsign}</div>` +
        `<div>${d.dep || "????"} → ${d.arr || "????"}</div>` +
        `<div>${d.actype || "—"} · ${d.alt}ft · ${d.gs}kt</div>`,
      style: {
        background: theme === "dark" ? "#111418" : "#ffffff",
        color: theme === "dark" ? "#e6edf3" : "#1b1f24",
        fontSize: "12px",
        padding: "6px 8px",
        borderRadius: "6px",
        boxShadow: "0 2px 8px rgba(0,0,0,.3)",
      },
    };
  };
}
