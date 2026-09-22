import * as React from "react";
import {CornerDownLeft, Search, Star, type LucideIcon} from "lucide-react";

import {cn} from "../lib/utils";
import {Modal} from "./modal";

export type CommandItem = {
  id: string;
  label: React.ReactNode;
  sublabel?: React.ReactNode;
  icon?: LucideIcon;
  onSelect: () => void;
  /** Whether the item is a favorite; with `onToggleStar`, the row shows a star (⌘⇧F toggles it). */
  starred?: boolean;
  onToggleStar?: () => void;
  /**
   * What this row *is*, when more than one row can stand for the same thing — a pinned Favorites row
   * and the source row it was starred from share an `entityId` but not an `id`. Un-starring removes
   * the pinned row, and this is what lets the highlight land on its twin instead of sliding
   * (VATUSA/OIS#312).
   */
  entityId?: string;
};

export type CommandGroup = { label: string; items: CommandItem[] };

/**
 * Whether a keydown is the favorite hotkey, ⌘⇧F / Ctrl+Shift+F. Shift is what keeps it clear of the
 * browser's own find (⌘F) and find-next (⌘G), so it is part of the match, not an afterthought.
 * It lives here, with the palette that fires it, so the open-palette and closed-palette paths cannot
 * drift — `web` imports this one rather than keeping a second copy (VATUSA/OIS#312).
 */
export function isFavoriteHotkey(e: Pick<KeyboardEvent, "metaKey" | "ctrlKey" | "shiftKey" | "key">): boolean {
  return (e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === "f";
}

/** A search scope the palette can narrow to (e.g. "Aircraft"); the first scope is the default. */
export type CommandScope = { id: string; label: string };

/** The scope `dir` steps (1 = next, -1 = previous) from `current`, wrapping at either end. */
export function cycleScope(ids: readonly string[], current: string, dir: 1 | -1): string {
  if (ids.length === 0) return current;
  const i = Math.max(0, ids.indexOf(current));
  return ids[(i + dir + ids.length) % ids.length];
}

/**
 * The index of the highlighted row. The palette tracks the highlighted *item* rather than a raw
 * index, so rebuilding `groups` — starring a row prepends one to the pinned Favorites group — keeps
 * the highlight on the row the user is actually on (VATUSA/OIS#312).
 *
 * When the highlighted row has *gone* the answer matters just as much, because ⌘⇧F is a toggle and
 * the reflex after one is to press it again. Resolving in three steps:
 *
 *   1. the row carrying `activeId`;
 *   2. else a row standing for the same thing — un-starring drops the pinned Favorites row, so the
 *      highlight moves to the source row it was starred from and the undo press puts it back;
 *   3. else `fallback`, the index last resolved, clamped — hold position rather than teleporting to
 *      the top. A row can vanish with no user input at all (traffic refetches every 15s), and
 *      collapsing to 0 pointed the next Enter at a flight nobody picked.
 */
export function activeIndex(
  items: readonly { id: string; entityId?: string }[],
  activeId: string | null,
  // What the highlight last resolved to. The vanished row is, by definition, no longer in `items`,
  // so its entity and position have to be remembered rather than looked up.
  last: { index: number; entityId?: string } = { index: 0 },
): number {
  if (items.length === 0) return 0;
  if (activeId == null) return 0;
  const exact = items.findIndex((i) => i.id === activeId);
  if (exact >= 0) return exact;
  const twin = last.entityId == null ? -1 : items.findIndex((i) => i.entityId === last.entityId);
  if (twin >= 0) return twin;
  return Math.min(Math.max(0, last.index), items.length - 1);
}

/**
 * A keyboard-first picker in a top-aligned modal: a search field over grouped results. It is
 * controlled — the caller owns `query` and passes already-filtered, ranked `groups` (so each source
 * can match however suits it: fuzzy callsigns, a server search, a static page list).
 * ↑/↓ move, Enter selects, Escape closes. With `scopes`, a chip row sits under the field: Tab /
 * Shift+Tab cycle the scope and Backspace on an empty query returns to the first (default) scope.
 * Items with `onToggleStar` carry a favorite star; ⌘⇧F / Ctrl+Shift+F toggles the highlighted one.
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
  scopes,
  scope,
  onScopeChange,
}: {
  open: boolean;
  onClose: () => void;
  query: string;
  onQueryChange: (query: string) => void;
  groups: CommandGroup[];
  placeholder?: string;
  empty?: React.ReactNode;
  footer?: React.ReactNode;
  scopes?: readonly CommandScope[];
  scope?: string;
  onScopeChange?: (scope: string) => void;
}) {
  const flat = React.useMemo(() => groups.flatMap((g) => g.items), [groups]);
  const [activeId, setActiveId] = React.useState<string | null>(null);
  // Written from an effect, so during the render where the highlighted row vanishes this still holds
  // where it was and what it stood for — which is exactly what `activeIndex` falls back to.
  const last = React.useRef<{ index: number; entityId?: string }>({ index: 0 });
  const active = activeIndex(flat, activeId, last.current);
  const listRef = React.useRef<HTMLDivElement>(null);
  // Highlight the row `step` away, by id — an index would go stale the next time `groups` changes.
  // Resolved from the previous id rather than `active` so batched keydowns don't both read one index.
  // `last` is redundant here — the effect below has already reconciled `activeId` to a live row by the
  // time a key arrives — and is passed only so this resolves exactly as the render does.
  const move = (step: 1 | -1) =>
    setActiveId(
      (id) => flat[Math.min(flat.length - 1, Math.max(0, activeIndex(flat, id, last.current) + step))]?.id ?? null,
    );

  React.useEffect(() => {
    // Only a real row is worth remembering. A list that renders empty for one tick (a traffic poll
    // returning nothing) would otherwise overwrite the held position with index 0, and the rows
    // coming back would land the highlight on the top one.
    if (flat[active] != null) last.current = { index: active, entityId: flat[active].entityId };
    // The highlight fell through to a twin or a held position: make that row the highlight. Otherwise
    // `activeId` still names the vanished row, and when a refetch brings it back the highlight jumps
    // off the row the user has been reading (VATUSA/OIS#339).
    const settled = flat[active]?.id;
    if (activeId != null && settled != null && settled !== activeId) setActiveId(settled);
  }, [active, flat, activeId]);

  React.useEffect(() => setActiveId(null), [query, open, scope]);

  React.useEffect(() => {
    listRef.current?.querySelector<HTMLElement>(`[data-index="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }, [active]);

  const select = (item: CommandItem | undefined) => {
    if (!item) return;
    onClose();
    item.onSelect();
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (isFavoriteHotkey(e)) {
      e.preventDefault();
      flat[active]?.onToggleStar?.();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      move(1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      move(-1);
    } else if (e.key === "Enter") {
      e.preventDefault();
      select(flat[active]);
    } else if (scopes?.length && onScopeChange && scope != null) {
      if (e.key === "Tab") {
        e.preventDefault();
        onScopeChange(cycleScope(scopes.map((s) => s.id), scope, e.shiftKey ? -1 : 1));
      } else if (e.key === "Backspace" && query === "" && scope !== scopes[0].id) {
        e.preventDefault();
        onScopeChange(scopes[0].id);
      }
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
      {scopes && scopes.length > 0 && scope != null && (
        <ScopeChips scopes={scopes} scope={scope} onScopeChange={onScopeChange} />
      )}
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
                  return (
                    <CommandRow
                      key={item.id}
                      item={item}
                      index={i}
                      active={i === active}
                      onActivate={() => setActiveId(item.id)}
                      onSelect={() => select(item)}
                    />
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

/**
 * One result row: the selection button and the favorite star side by side. The star is a **sibling**
 * of the row button, never a descendant — a nested interactive role has no defined behaviour, so
 * assistive tech may expose one control, the wrong one, or neither (VATUSA/OIS#336). Being siblings
 * is also what keeps a star click from selecting the row. The active tint sits on the container so
 * the star stays inside the highlight, and the Enter hint sits inside the selection button so the
 * whole row minus the star still selects.
 */
export function CommandRow({
  item,
  index,
  active,
  onActivate,
  onSelect,
}: {
  item: CommandItem;
  index: number;
  active: boolean;
  onActivate: () => void;
  onSelect: () => void;
}) {
  const Icon = item.icon;
  return (
    <div
      data-index={index}
      onMouseMove={onActivate}
      className={cn(
        "flex w-full items-center rounded-sm text-sm",
        active ? "bg-panel-2 text-ink" : "text-ink-2",
      )}
    >
      <button
        type="button"
        onClick={onSelect}
        // The whole row bar the star selects, padding included — a gutter outside the button would
        // be a strip of the row that highlights on hover but does nothing when clicked.
        className="flex min-w-0 flex-1 items-center gap-2.5 px-2.5 py-2 text-left"
      >
        {Icon && <Icon className={cn("size-4 shrink-0", active ? "text-brand-ink" : "text-ink-3")} />}
        <span className="min-w-0 flex-1 truncate">{item.label}</span>
        {item.sublabel && <span className="shrink-0 font-mono text-xs text-ink-3">{item.sublabel}</span>}
        {active && <CornerDownLeft className="size-3.5 shrink-0 text-ink-3" />}
      </button>
      {item.onToggleStar && (item.starred || active) && (
        <button
          type="button"
          aria-label={item.starred ? "Remove from favorites" : "Add to favorites"}
          aria-pressed={!!item.starred}
          // Keep focus in the search field so the keyboard keeps working after a click.
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => item.onToggleStar?.()}
          className="mr-2.5 shrink-0 rounded-sm p-0.5 text-ink-3 hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <Star className={cn("size-3.5", item.starred && "fill-current text-brand-ink")} />
        </button>
      )}
    </div>
  );
}

/**
 * The palette's scope row: one pill per scope, the active one tinted. Clicking keeps focus in the
 * search field so the keyboard keeps working. A standard radiogroup otherwise — only the checked
 * chip is tabbable and ←/→ move the selection — with a live region, since the field usually holds
 * focus and a screen reader would otherwise never hear the scope change.
 */
export function ScopeChips({
  scopes,
  scope,
  onScopeChange,
}: {
  scopes: readonly CommandScope[];
  scope: string;
  onScopeChange?: (scope: string) => void;
}) {
  const ref = React.useRef<HTMLDivElement>(null);
  const active = scopes.find((s) => s.id === scope);

  const move = (dir: 1 | -1) => {
    const next = cycleScope(scopes.map((s) => s.id), scope, dir);
    onScopeChange?.(next);
    ref.current?.querySelector<HTMLElement>(`[data-scope="${next}"]`)?.focus();
  };

  return (
    <div className="border-b border-line">
      <div ref={ref} role="radiogroup" aria-label="Search scope" className="flex flex-wrap gap-1.5 px-3 py-2">
        {scopes.map((s) => {
          const on = s.id === scope;
          return (
            <button
              key={s.id}
              type="button"
              role="radio"
              aria-checked={on}
              data-scope={s.id}
              tabIndex={on ? 0 : -1}
              // Keep focus in the search field so the keyboard keeps working after a click.
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => onScopeChange?.(s.id)}
              onKeyDown={(e) => {
                if (e.key === "ArrowRight" || e.key === "ArrowLeft") {
                  e.preventDefault();
                  move(e.key === "ArrowRight" ? 1 : -1);
                }
              }}
              className={cn(
                "inline-flex h-7 items-center rounded-full border px-2.5 text-xs",
                on ? "border-brand/40 bg-brand-soft text-ink" : "border-line bg-panel-2 text-ink-2 hover:text-ink",
              )}
            >
              {s.label}
            </button>
          );
        })}
      </div>
      <span className="sr-only" aria-live="polite">
        {active ? `${active.label} scope` : ""}
      </span>
    </div>
  );
}
