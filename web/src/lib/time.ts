/** Format an ISO timestamp as NTML "DD/HHMMz" (day-of-month + Zulu time). */
export function formatZulu(iso: string | null | undefined): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "—";
  const dd = String(d.getUTCDate()).padStart(2, "0");
  const hh = String(d.getUTCHours()).padStart(2, "0");
  const mm = String(d.getUTCMinutes()).padStart(2, "0");
  return `${dd}/${hh}${mm}z`;
}

/** Format an ISO timestamp as a bare Zulu time "HHMMz". */
export function hhmmZulu(iso: string | null | undefined): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "—";
  const hh = String(d.getUTCHours()).padStart(2, "0");
  const mm = String(d.getUTCMinutes()).padStart(2, "0");
  return `${hh}${mm}z`;
}

/**
 * Parse NTML "DD/HHMMz" (day-of-month + Zulu time) into an ISO timestamp, relative to
 * the current UTC month. If the day has already passed this month it rolls to the next
 * month. Returns null on empty or malformed input.
 */
export function parseZulu(input: string): string | null {
  const m = /^(\d{1,2})\/(\d{2})(\d{2})z?$/i.exec(input.trim());
  if (!m) return null;
  const day = Number(m[1]);
  const hour = Number(m[2]);
  const minute = Number(m[3]);
  if (day < 1 || day > 31 || hour > 23 || minute > 59) return null;

  const now = new Date();
  let year = now.getUTCFullYear();
  let month = now.getUTCMonth();
  let ts = Date.UTC(year, month, day, hour, minute);
  // If the entry is more than a day in the past, assume it's for next month.
  if (ts < now.getTime() - 24 * 3600 * 1000) {
    month += 1;
    if (month > 11) {
      month = 0;
      year += 1;
    }
    ts = Date.UTC(year, month, day, hour, minute);
  }
  const d = new Date(ts);
  // Reject impossible days (e.g. 31 in a 30-day month, which JS would roll forward).
  if (d.getUTCDate() !== day) return null;
  return d.toISOString();
}

/**
 * Parse a bare Zulu clock time "HHMMz" into an ISO timestamp on today's UTC date,
 * nudged to the nearest occurrence around `now`. Returns null on malformed input.
 */
export function parseHhmm(input: string, now = Date.now()): string | null {
  const m = /^(\d{2})(\d{2})z?$/i.exec(input.trim());
  if (!m) return null;
  const h = Number(m[1]);
  const min = Number(m[2]);
  if (h > 23 || min > 59) return null;
  const d = new Date(now);
  d.setUTCHours(h, min, 0, 0);
  let t = d.getTime();
  if (t < now - 12 * 3600_000) t += 24 * 3600_000;
  else if (t - now > 12 * 3600_000) t -= 24 * 3600_000;
  return new Date(t).toISOString();
}

/** Compact relative time, e.g. "22h ago". */
export function timeAgo(iso: string): string {
  const then = new Date(iso).getTime();
  const seconds = Math.max(0, Math.floor((Date.now() - then) / 1000));
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}d ago`;
  return new Date(iso).toLocaleDateString();
}
