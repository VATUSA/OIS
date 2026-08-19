import {useMemo, useState} from "react";
import {Input} from "@ois/ui";

import {facilityKindLabel, useFacilityDirectory, type FlowFacility} from "@/lib/facilities";

export type FacilityPick = { id: string; kind: "artcc" | "tracon" };

/** Rank a facility against a query; lower = better, -1 = no match. */
function matchRank(q: string, f: FlowFacility): number {
  const id = f.id.toUpperCase();
  const name = (f.name ?? "").toUpperCase();
  if (id.startsWith(q)) return 0;
  if (id.includes(q)) return 1;
  if (name.startsWith(q)) return 2;
  if (name.includes(q)) return 3;
  return -1;
}

/**
 * Type-to-filter facility picker — ARTCCs (center) and TRACONs (approach). Self-contained (loads the
 * facility directory). Ranks by id/name; ARTCCs float above TRACONs on equal rank (fewer, more common).
 */
export function FacilityCombobox({
  onSelect,
  placeholder = "Facility — ZDC, PCT, ZNY…",
  autoFocus,
}: {
  onSelect: (pick: FacilityPick) => void;
  placeholder?: string;
  autoFocus?: boolean;
}) {
  const dir = useFacilityDirectory();
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);

  const all = dir.data ?? [];
  const q = query.trim().toUpperCase();
  const matches = useMemo(() => {
    const kindRank = (k: string) => (k === "artcc" ? 0 : 1);
    return (
      q
        ? all
            .map((f) => ({ f, rank: matchRank(q, f) }))
            .filter((x) => x.rank >= 0)
            .sort(
              (x, y) =>
                x.rank - y.rank ||
                kindRank(x.f.kind) - kindRank(y.f.kind) ||
                x.f.id.localeCompare(y.f.id),
            )
            .map((x) => x.f)
        : all.slice().sort((a, b) => kindRank(a.kind) - kindRank(b.kind) || a.id.localeCompare(b.id))
    ).slice(0, 8);
  }, [all, q]);

  const pick = (f: FlowFacility) => {
    onSelect({ id: f.id, kind: f.kind === "artcc" ? "artcc" : "tracon" });
    setQuery("");
    setOpen(false);
  };

  return (
    <div className="relative w-72">
      <Input
        // eslint-disable-next-line jsx-a11y/no-autofocus
        autoFocus={autoFocus}
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
            pick(matches[0]);
          } else if (e.key === "Escape") {
            setOpen(false);
          }
        }}
      />
      {open && matches.length > 0 && (
        <ul className="absolute z-20 mt-1 max-h-64 w-full overflow-auto rounded-md border bg-popover p-1 shadow-md">
          {matches.map((f) => (
            <li key={f.id}>
              <button
                type="button"
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => pick(f)}
                className="flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left text-sm transition-colors hover:bg-accent hover:text-accent-foreground"
              >
                <span className="w-12 shrink-0 font-mono font-medium">{f.id}</span>
                <span
                  className={
                    "shrink-0 rounded px-1.5 py-0.5 text-[10px] font-medium uppercase " +
                    (f.kind === "artcc" ? "bg-primary/15 text-primary" : "bg-amber-500/15 text-amber-600 dark:text-amber-400")
                  }
                >
                  {facilityKindLabel(f.kind)}
                </span>
                {f.name && <span className="truncate text-muted-foreground">{f.name}</span>}
                <span className="ml-auto shrink-0 text-xs tabular-nums text-muted-foreground/60">
                  {f.airports.length}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
