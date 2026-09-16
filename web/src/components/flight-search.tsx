import {useEffect, useMemo, useRef, useState} from "react";

import {cn} from "@ois/ui";
import {Search} from "lucide-react";

import {rankAircraft, type SearchableAircraft} from "@/lib/fuzzy";

type Props<T extends SearchableAircraft> = {
  /** Live traffic to search over. */
  aircraft: readonly T[];
  /** Called with an uppercased callsign when a flight is picked (or the raw query on Enter). */
  onSelect: (callsign: string) => void;
  placeholder?: string;
  autoFocus?: boolean;
  /** Wrapper class (e.g. width). */
  className?: string;
  /** Compact translucent styling for the map corner vs. a normal form field. */
  variant?: "default" | "overlay";
  limit?: number;
};

/**
 * Fuzzy find-as-you-type flight picker. Ranks the live-traffic list on every keystroke and
 * shows a keyboard-navigable dropdown (↑/↓/↵/esc); Enter with nothing highlighted submits the
 * raw text, preserving "type an exact callsign and go". Shared by the FCA map and pilot page.
 */
export function FlightSearch<T extends SearchableAircraft>({
  aircraft,
  onSelect,
  placeholder = "Search flights…",
  autoFocus,
  className,
  variant = "default",
  limit = 8,
}: Props<T>) {
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const rootRef = useRef<HTMLDivElement>(null);

  const hits = useMemo(
    () => rankAircraft(query, aircraft, limit),
    [query, aircraft, limit],
  );

  // Reset the highlighted row whenever the result set changes.
  useEffect(() => {
    setActive(0);
  }, [query]);

  // Close the dropdown on an outside click.
  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open]);

  const choose = (callsign: string) => {
    const v = callsign.trim().toUpperCase();
    if (!v) return;
    onSelect(v);
    setQuery(v);
    setOpen(false);
  };

  const showList = open && query.trim().length > 0 && hits.length > 0;
  const overlay = variant === "overlay";

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setOpen(true);
      setActive((i) => Math.min(i + 1, hits.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((i) => Math.max(i - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (showList && hits[active]) choose(hits[active].ac.callsign);
      else choose(query);
    } else if (e.key === "Escape") {
      setOpen(false);
    }
  };

  return (
    <div ref={rootRef} className={cn("relative", className)}>
      <div
        className={cn(
          "flex items-center gap-1.5",
          overlay
            ? "h-8 rounded-full border border-line bg-panel px-3 focus-within:ring-2 focus-within:ring-ring"
            : "h-9 rounded-full border border-line bg-panel-2 px-3.5 focus-within:ring-2 focus-within:ring-ring",
        )}
      >
        <Search className="size-3.5 shrink-0 text-ink-3" />
        <input
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setOpen(true);
          }}
          onFocus={() => setOpen(true)}
          onKeyDown={onKeyDown}
          placeholder={placeholder}
          aria-label="Search flights by callsign"
          role="combobox"
          aria-expanded={showList}
          aria-autocomplete="list"
          autoFocus={autoFocus}
          className={cn(
            "w-full bg-transparent font-mono uppercase text-ink outline-none placeholder:normal-case placeholder:font-sans placeholder:text-ink-3",
            overlay ? "h-full w-32 text-xs" : "text-sm",
          )}
        />
      </div>

      {showList && (
        <ul
          role="listbox"
          className={cn(
            "absolute z-[1000] mt-1 max-h-72 min-w-56 overflow-auto rounded-sm border border-line bg-panel p-1 text-ink",
            overlay ? "right-0" : "w-full",
          )}
        >
          {hits.map((h, i) => (
            <li key={h.ac.callsign} role="option" aria-selected={i === active}>
              <button
                type="button"
                onMouseEnter={() => setActive(i)}
                onMouseDown={(e) => {
                  e.preventDefault(); // keep focus; select before blur closes the list
                  choose(h.ac.callsign);
                }}
                className={cn(
                  "flex w-full items-center justify-between gap-3 rounded-xs px-2 py-1.5 text-left",
                  i === active ? "bg-brand-soft text-ink" : "hover:bg-panel-2",
                )}
              >
                <span className="font-mono text-xs font-semibold tracking-wide">
                  <Highlight text={h.ac.callsign} positions={h.positions} />
                </span>
                <span className="shrink-0 font-mono text-[10px] text-ink-3">
                  {h.ac.dep || "?"}→{h.ac.arr || "?"}
                  {h.ac.actype ? ` · ${h.ac.actype}` : ""}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** Bold the fuzzy-matched characters of the callsign. */
function Highlight({text, positions}: {text: string; positions: number[]}) {
  if (positions.length === 0) return <>{text}</>;
  const set = new Set(positions);
  return (
    <>
      {text.split("").map((ch, i) => (
        <span key={i} className={set.has(i) ? "text-brand-ink" : undefined}>
          {ch}
        </span>
      ))}
    </>
  );
}
