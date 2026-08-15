/**
 * Lightweight fuzzy subsequence matcher (a tiny fzf) for callsign search — no dependency,
 * runs fine over the full ~2k live-traffic list on every keystroke.
 */
export type FuzzyResult = { score: number; positions: number[] };

function isAlnum(c: number) {
  return (c >= 48 && c <= 57) || (c >= 65 && c <= 90);
}

/**
 * Match `query` against `text` as an ordered subsequence. Returns a score (higher is better)
 * and the matched character indices in `text` (for highlighting), or null if it doesn't match.
 */
export function fuzzyMatch(query: string, text: string): FuzzyResult | null {
  const q = query.toUpperCase();
  const t = text.toUpperCase();
  if (q.length === 0) return { score: 0, positions: [] };
  if (q.length > t.length) return null;

  let qi = 0;
  let score = 0;
  let prevMatch = -2;
  const positions: number[] = [];
  for (let ti = 0; ti < t.length && qi < q.length; ti++) {
    if (t.charCodeAt(ti) === q.charCodeAt(qi)) {
      let s = 1;
      if (ti === 0)
        s += 5; // matches at the very start
      else if (!isAlnum(t.charCodeAt(ti - 1)))
        s += 3; // matches right after a separator
      if (prevMatch === ti - 1) s += 4; // contiguous run
      score += s;
      positions.push(ti);
      prevMatch = ti;
      qi++;
    }
  }
  if (qi < q.length) return null; // ran out of text before matching all of query
  // Prefer tighter matches and ones that start earlier.
  score -= (t.length - q.length) * 0.5;
  score -= positions[0] * 0.5;
  return { score, positions };
}

export type SearchableAircraft = {
  callsign: string;
  dep?: string;
  arr?: string;
  actype?: string;
};

export type AircraftHit<T> = { ac: T; score: number; positions: number[] };

/**
 * Rank live aircraft against a query. Callsign matches dominate; a query can also match by
 * route (dep/arr) or aircraft type, so "KLAX" or "B738" surface relevant flights too.
 * `positions` are the matched indices within the callsign (empty when the hit came from
 * route/type only), for highlighting.
 */
export function rankAircraft<T extends SearchableAircraft>(
  query: string,
  list: readonly T[],
  limit = 8,
): AircraftHit<T>[] {
  const q = query.trim();
  if (!q) return [];
  const hits: AircraftHit<T>[] = [];
  for (const ac of list) {
    const cs = fuzzyMatch(q, ac.callsign);
    let score = cs ? cs.score * 3 : Number.NEGATIVE_INFINITY;
    const positions = cs ? cs.positions : [];

    const route = fuzzyMatch(q, `${ac.dep ?? ""}${ac.arr ?? ""}`);
    if (route) score = Math.max(score, route.score);
    const type = ac.actype ? fuzzyMatch(q, ac.actype) : null;
    if (type) score = Math.max(score, type.score * 0.8);

    if (score > Number.NEGATIVE_INFINITY) hits.push({ ac, score, positions });
  }
  hits.sort(
    (a, b) => b.score - a.score || a.ac.callsign.localeCompare(b.ac.callsign),
  );
  return hits.slice(0, limit);
}
