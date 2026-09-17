import type {CommandScope} from "@ois/ui";

/**
 * The command-search scopes, in chip order; `all` (the default) blends every group. `label` is the
 * chip and the group heading; `noun` is the same scope inside a sentence ("No TMIs match."), which
 * is why it is written out rather than lower-cased from the label.
 */
export const SCOPES = [
  { id: "all", label: "All", noun: "results" },
  { id: "aircraft", label: "Aircraft", noun: "aircraft" },
  { id: "tmis", label: "TMIs", noun: "TMIs" },
  { id: "events", label: "Events", noun: "events" },
  { id: "dashboards", label: "Dashboards", noun: "dashboards" },
  { id: "airports", label: "Airport data", noun: "airport data" },
  { id: "pages", label: "Pages", noun: "pages" },
] as const satisfies readonly (CommandScope & { noun: string })[];

export type ScopeId = (typeof SCOPES)[number]["id"];

/**
 * A typed scope prefix: `@<prefix> ` (then the rest of the query) where `<prefix>` uniquely starts
 * one available scope's id or label, case-insensitively — `@tmi ` or `@airc KDEN`. Returns the scope
 * and the query with the prefix stripped, or null when the query has no complete, unambiguous prefix.
 */
export function parseScopePrefix(
  query: string,
  available: readonly CommandScope[],
): { scope: string; rest: string } | null {
  const m = /^@(\S+)\s(.*)$/s.exec(query);
  if (!m) return null;
  const prefix = m[1].toLowerCase();
  const hits = available.filter(
    (s) => s.id.toLowerCase().startsWith(prefix) || s.label.toLowerCase().startsWith(prefix),
  );
  return hits.length === 1 ? { scope: hits[0].id, rest: m[2] } : null;
}

/** The pages an ICAO-shaped query offers, in row order; the first also shows in the blended view. */
export const ICAO_ROW_PAGES = [
  { to: "/ops/airport", suffix: "airport", sublabel: "Operations · Airport" },
  { to: "/admin/planning/airport-configs", suffix: "airport configs", sublabel: "Planning" },
  { to: "/admin/planning/airport-surface", suffix: "airport surface", sublabel: "Planning" },
] as const;

export type IcaoRow = {
  to: (typeof ICAO_ROW_PAGES)[number]["to"];
  label: string;
  sublabel: string;
  search: { icao: string };
};

/**
 * The command-search rows for an ICAO-shaped query, or none when `icao` isn't one. Each row carries
 * the airport in `search` — a row is labelled with an airport, so selecting it must open *that*
 * airport rather than the page's empty picker (VATUSA/OIS#311).
 */
export function icaoRows(icao: string): IcaoRow[] {
  if (!/^[A-Z0-9]{3,4}$/.test(icao)) return [];
  return ICAO_ROW_PAGES.map((p) => ({
    to: p.to,
    label: `${icao} ${p.suffix}`,
    sublabel: p.sublabel,
    search: { icao },
  }));
}

/** Where a TMI row goes: the TMU restrictions tab, filtered to the TMI's requesting facility. */
export function tmiRow(tmi: { requesting: string }) {
  return { to: "/ops/tmu", search: { tab: "restrictions", facility: tmi.requesting } } as const;
}
