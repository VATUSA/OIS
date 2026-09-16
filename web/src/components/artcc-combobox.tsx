import {useState} from "react";
import {Input} from "@ois/ui";

import {useFacilities} from "@/lib/admin";

export type Artcc = { id: string; name: string };

/** Rank an ARTCC against a query; lower = better, -1 = no match. */
function matchRank(q: string, a: Artcc): number {
  const id = a.id.toUpperCase();
  const name = a.name.toUpperCase();
  if (id.startsWith(q)) return 0;
  if (id.includes(q)) return 1;
  if (name.startsWith(q)) return 2;
  if (name.includes(q)) return 3;
  return -1;
}

/**
 * Type-to-filter ARTCC picker. Self-contained (loads the ARTCC directory itself);
 * offers only active ARTCCs not already in `exclude`.
 */
export function ArtccCombobox({
  exclude,
  onSelect,
  placeholder = "Add ARTCC — code or name…",
}: {
  exclude: string[];
  onSelect: (id: string) => void;
  placeholder?: string;
}) {
  const facilities = useFacilities();
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);

  const available: Artcc[] = (facilities.data ?? [])
    .filter((f) => f.active && !exclude.includes(f.id))
    .map((f) => ({ id: f.id, name: f.name }));

  const q = query.trim().toUpperCase();
  const matches = (
    q
      ? available
          .map((a) => ({ a, rank: matchRank(q, a) }))
          .filter((x) => x.rank >= 0)
          .sort((x, y) => x.rank - y.rank || x.a.id.localeCompare(y.a.id))
          .map((x) => x.a)
      : available
  ).slice(0, 8);

  const pick = (id: string) => {
    onSelect(id);
    setQuery("");
    setOpen(false);
  };

  return (
    <div className="relative w-72">
      <Input
        className="font-mono uppercase"
        placeholder={placeholder}
        value={query}
        onChange={(e) => {
          setQuery(e.target.value);
          setOpen(true);
        }}
        onFocus={() => setOpen(true)}
        onBlur={() => setTimeout(() => setOpen(false), 120)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && matches[0]) {
            e.preventDefault();
            pick(matches[0].id);
          } else if (e.key === "Escape" && open) {
            // Closing the list shouldn't also close an enclosing Modal.
            e.preventDefault();
            setOpen(false);
          }
        }}
      />
      {open && matches.length > 0 && (
        <ul className="absolute z-20 mt-1 max-h-64 w-full overflow-auto rounded-sm border border-line bg-panel-2 p-1">
          {matches.map((a) => (
            <li key={a.id}>
              <button
                type="button"
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => pick(a.id)}
                className="flex w-full items-center gap-2 rounded-xs px-2 py-1.5 text-left text-sm text-ink transition-colors hover:bg-chip"
              >
                <span className="w-10 shrink-0 font-mono font-semibold">{a.id}</span>
                <span className="truncate text-ink-2">{a.name}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
