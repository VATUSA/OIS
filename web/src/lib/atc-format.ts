/** Shared formatting for ATC positions (used by the map hover tooltip and the dashboard ATC widget). */

/** VATSIM controller rating id → label. */
export const RATINGS: Record<number, string> = {
  2: "S1", 3: "S2", 4: "S3", 5: "C1", 6: "C2", 7: "C3", 8: "I1", 9: "I2", 10: "I3", 11: "SUP", 12: "ADM",
};

export const ratingLabel = (rating: number): string => RATINGS[rating] ?? "";

/** How long the controller has been on position, e.g. "2h14m" (blank if unknown/parse-fails). */
export function onlineFor(iso: string): string {
  if (!iso) return "";
  const t = Date.parse(iso);
  if (!Number.isFinite(t)) return "";
  const mins = Math.floor((Date.now() - t) / 60000);
  if (mins < 0) return "";
  const h = Math.floor(mins / 60);
  const m = mins % 60;
  return h > 0 ? `${h}h${m}m` : `${m}m`;
}
