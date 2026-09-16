import * as React from "react";
import {CornerDownLeft, Search, type LucideIcon} from "lucide-react";

import {cn} from "../lib/utils";
import {Modal} from "./modal";

export type CommandItem = {
  id: string;
  label: React.ReactNode;
  sublabel?: React.ReactNode;
  icon?: LucideIcon;
  onSelect: () => void;
};

export type CommandGroup = { label: string; items: CommandItem[] };

/**
 * A keyboard-first picker in a top-aligned modal: a search field over grouped results. It is
 * controlled — the caller owns `query` and passes already-filtered, ranked `groups` (so each source
 * can match however suits it: fuzzy callsigns, a server search, a static page list).
 * ↑/↓ move, Enter selects, Escape closes.
 */
export function CommandPalette({
  open,
  onClose,
  query,
  onQueryChange,
  groups,
  placeholder = "Search…",
  empty = "No results.",
  footer,
}: {
  open: boolean;
  onClose: () => void;
  query: string;
  onQueryChange: (query: string) => void;
  groups: CommandGroup[];
  placeholder?: string;
  empty?: React.ReactNode;
  footer?: React.ReactNode;
}) {
  const flat = React.useMemo(() => groups.flatMap((g) => g.items), [groups]);
  const [active, setActive] = React.useState(0);
  const listRef = React.useRef<HTMLDivElement>(null);

  React.useEffect(() => setActive(0), [query, open]);

  React.useEffect(() => {
    listRef.current?.querySelector<HTMLElement>(`[data-index="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }, [active]);

  const select = (item: CommandItem | undefined) => {
    if (!item) return;
    onClose();
    item.onSelect();
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((i) => Math.min(flat.length - 1, i + 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((i) => Math.max(0, i - 1));
    } else if (e.key === "Enter") {
      e.preventDefault();
      select(flat[active]);
    }
  };

  let index = -1;
  return (
    <Modal open={open} onClose={onClose} size="md" placement="top" aria-label={placeholder}>
      <div className="flex items-center gap-2.5 border-b border-line px-4">
        <Search className="size-4 shrink-0 text-ink-3" />
        <input
          autoFocus
          value={query}
          onChange={(e) => onQueryChange(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder={placeholder}
          className="h-12 w-full bg-transparent text-sm text-ink outline-none placeholder:text-ink-3"
        />
        <kbd className="rounded-[4px] border border-line px-1.5 font-mono text-[10px] text-ink-3">esc</kbd>
      </div>
      <div ref={listRef} className="max-h-[55vh] overflow-y-auto p-2">
        {flat.length === 0 ? (
          <p className="px-3 py-6 text-center text-sm text-ink-3">{empty}</p>
        ) : (
          groups
            .filter((g) => g.items.length > 0)
            .map((g) => (
              <div key={g.label} className="mb-1">
                <div className="px-2 pb-1 pt-2 text-[10.5px] font-semibold uppercase tracking-[0.09em] text-ink-3">
                  {g.label}
                </div>
                {g.items.map((item) => {
                  index += 1;
                  const i = index;
                  const on = i === active;
                  const Icon = item.icon;
                  return (
                    <button
                      key={item.id}
                      type="button"
                      data-index={i}
                      onMouseMove={() => setActive(i)}
                      onClick={() => select(item)}
                      className={cn(
                        "flex w-full items-center gap-2.5 rounded-sm px-2.5 py-2 text-left text-sm",
                        on ? "bg-panel-2 text-ink" : "text-ink-2",
                      )}
                    >
                      {Icon && <Icon className={cn("size-4 shrink-0", on ? "text-brand-ink" : "text-ink-3")} />}
                      <span className="min-w-0 flex-1 truncate">{item.label}</span>
                      {item.sublabel && <span className="shrink-0 font-mono text-xs text-ink-3">{item.sublabel}</span>}
                      {on && <CornerDownLeft className="size-3.5 shrink-0 text-ink-3" />}
                    </button>
                  );
                })}
              </div>
            ))
        )}
      </div>
      {footer && <div className="border-t border-line px-4 py-2 text-xs text-ink-3">{footer}</div>}
    </Modal>
  );
}
