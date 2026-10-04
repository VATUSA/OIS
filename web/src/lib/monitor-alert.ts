/**
 * The Monitor's alert ladder, as the backend serializes it (`feed::monitor_alert::SectorAlert`,
 * #600). Red: the airborne peak alone exceeds the sector's MAP. Amber: only airborne + proposed does.
 * Green: neither.
 */
export type SectorAlert = "green" | "amber" | "red";

/**
 * Each state's load-level token (DESIGN.md § Domain tokens) — the status semantics, never the accent,
 * and never a literal colour: `monitor-colors.guard.test.ts` rejects one anywhere in Monitor code.
 */
export const ALERT_LEVEL = {
  green: "level-ok",
  amber: "level-watch",
  red: "level-over",
} as const satisfies Record<SectorAlert, string>;

/** Text utility per state, e.g. `text-level-over`. Spelled out so Tailwind sees every class. */
export const alertTextClass: Record<SectorAlert, string> = {
  green: "text-level-ok",
  amber: "text-level-watch",
  red: "text-level-over",
};

/** CSS custom property per state, for `useTokens` where `var()` can't reach (deck.gl, charts). */
export const alertToken: Record<SectorAlert, `--${(typeof ALERT_LEVEL)[SectorAlert]}`> = {
  green: `--${ALERT_LEVEL.green}`,
  amber: `--${ALERT_LEVEL.amber}`,
  red: `--${ALERT_LEVEL.red}`,
};
