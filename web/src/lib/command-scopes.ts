import type {CommandScope} from "@ois/ui";

/** The command-search scopes, in chip order; `all` (the default) blends every group. */
export const SCOPES = [
  { id: "all", label: "All" },
  { id: "aircraft", label: "Aircraft" },
  { id: "tmis", label: "TMIs" },
  { id: "events", label: "Events" },
  { id: "dashboards", label: "Dashboards" },
  { id: "airports", label: "Airport data" },
  { id: "pages", label: "Pages" },
] as const satisfies readonly CommandScope[];

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
