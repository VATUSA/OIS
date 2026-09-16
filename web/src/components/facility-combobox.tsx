import {useMemo, useState} from "react";
import {cn, Input, StatusPill} from "@ois/ui";

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
  className,
  inputClassName,
}: {
  onSelect: (pick: FacilityPick) => void;
  placeholder?: string;
  autoFocus?: boolean;
  /** Override the wrapper width (default `w-72`). */
  className?: string;
  /** Extra classes on the input (e.g. `h-8` to match a compact field). */
  inputClassName?: string;
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
    <div className={cn("relative w-72", className)}>
      <Input
        // eslint-disable-next-line jsx-a11y/no-autofocus
        autoFocus={autoFocus}
        className={cn("font-mono uppercase", inputClassName)}
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
        <ul className="absolute z-20 mt-1 max-h-64 w-full overflow-auto rounded-sm border border-line bg-panel-2 p-1">
          {matches.map((f) => (
            <li key={f.id}>
              <button
                type="button"
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => pick(f)}
                className="flex w-full items-center gap-2 rounded-xs px-2 py-1.5 text-left text-sm text-ink transition-colors hover:bg-chip"
              >
                <span className="w-12 shrink-0 font-mono font-semibold">{f.id}</span>
                <StatusPill tone={f.kind === "artcc" ? "brand" : "neutral"} className="shrink-0 px-1.5 py-0 text-[10px] uppercase">
                  {facilityKindLabel(f.kind)}
                </StatusPill>
                {f.name && <span className="truncate text-ink-2">{f.name}</span>}
                <span className="ml-auto shrink-0 font-mono text-xs tabular-nums text-ink-3">
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
