import {useMemo, useState} from "react";
import {Input, StatusPill, cn} from "@ois/ui";
import {ChevronDown, ChevronRight} from "lucide-react";

/** Per-item scope choice: grant nationally, or to the listed ARTCCs. */
export type ScopeSel = { national: boolean; artccs: string[] };

/** What an actor may delegate for one item: nationally, and/or which ARTCCs. */
export type ScopeBounds = { national: boolean; artccs: string[] };

/** One selectable item (a permission or a role) with the scope bounds it can be granted at. */
export type ScopeItem = { name: string; bounds: ScopeBounds };

/** A name → chosen scope map (the working selection for one editor). */
export type ScopeSelection = Map<string, ScopeSel>;

/** The default scope when an item is first checked: national if allowed, else all its ARTCCs. */
export function defaultScope(bounds: ScopeBounds): ScopeSel {
  return bounds.national
    ? { national: true, artccs: [] }
    : { national: false, artccs: [...bounds.artccs] };
}

/** A scoped selection is valid to submit once every entry has a scope (national or ≥1 ARTCC). */
export function selectionIsValid(selection: ScopeSelection): boolean {
  for (const s of selection.values()) {
    if (!s.national && s.artccs.length === 0) return false;
  }
  return true;
}

function Chip({
  label,
  active,
  disabled,
  onClick,
}: {
  label: string;
  active: boolean;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className={cn(
        "rounded-full border px-2 py-0.5 font-mono text-xs font-semibold transition-colors",
        active
          ? "border-brand/40 bg-brand-soft text-brand-ink"
          : "border-line bg-panel-2 text-ink-2 hover:bg-chip hover:text-ink",
        disabled && "cursor-not-allowed opacity-50",
      )}
    >
      {label}
    </button>
  );
}

/** The scope controls shown under a checked item: National (if allowed) and/or specific ARTCCs. */
export function ScopeChips({
  bounds,
  sel,
  facilities,
  disabled,
  onChange,
}: {
  bounds: ScopeBounds;
  sel: ScopeSel;
  facilities: { id: string; name: string }[];
  disabled?: boolean;
  onChange: (next: ScopeSel) => void;
}) {
  // A national holder may grant nationally OR narrow to any ARTCC; a scoped holder is limited to
  // the ARTCCs they actually hold the item in.
  const options = bounds.national ? facilities.map((f) => f.id) : bounds.artccs;
  const toggleArtcc = (id: string) => {
    const has = sel.artccs.includes(id);
    onChange({
      national: false,
      artccs: has ? sel.artccs.filter((a) => a !== id) : [...sel.artccs, id],
    });
  };

  return (
    <div className="ml-6 mt-1 flex flex-wrap items-center gap-1.5">
      {bounds.national && (
        <Chip
          label="National"
          active={sel.national}
          disabled={disabled}
          onClick={() => onChange({ national: true, artccs: [] })}
        />
      )}
      {bounds.national && <span className="text-xs text-ink-3">or</span>}
      {options.map((id) => (
        <Chip
          key={id}
          label={id}
          active={!sel.national && sel.artccs.includes(id)}
          disabled={disabled}
          onClick={() => toggleArtcc(id)}
        />
      ))}
      {!sel.national && sel.artccs.length === 0 && (
        <span className="text-xs text-danger">pick at least one ARTCC</span>
      )}
    </div>
  );
}

/**
 * Grouped, collapsible, searchable permission tree with a per-item scope control (National /
 * specific ARTCCs). Shared by the user access editor and the API-key permission picker so both
 * present and scope permissions identically. Items are grouped by their first `.`-segment (domain).
 */
export function PermissionScopeTree({
  items,
  facilities,
  selection,
  disabled,
  onChange,
}: {
  items: ScopeItem[];
  facilities: { id: string; name: string }[];
  selection: ScopeSelection;
  disabled?: boolean;
  onChange: (next: ScopeSelection) => void;
}) {
  const [q, setQ] = useState("");
  const [open, setOpen] = useState<Set<string>>(() => new Set());

  const groups = useMemo(() => {
    const needle = q.trim().toLowerCase();
    const byDomain = new Map<string, ScopeItem[]>();
    for (const it of items) {
      if (needle && !it.name.toLowerCase().includes(needle)) continue;
      const domain = it.name.split(".")[0];
      const list = byDomain.get(domain) ?? [];
      list.push(it);
      byDomain.set(domain, list);
    }
    return [...byDomain.entries()].sort(([a], [b]) => a.localeCompare(b));
  }, [items, q]);

  const toggle = (it: ScopeItem, on: boolean) => {
    const next = new Map(selection);
    if (on) next.set(it.name, defaultScope(it.bounds));
    else next.delete(it.name);
    onChange(next);
  };
  const setScope = (name: string, s: ScopeSel) => {
    const next = new Map(selection);
    next.set(name, s);
    onChange(next);
  };

  return (
    <div className="flex flex-col gap-2">
      <Input
        value={q}
        onChange={(e) => setQ(e.target.value)}
        placeholder="Filter permissions…"
        className="h-8"
      />
      <div className="max-h-80 overflow-y-auto rounded-md border border-line bg-panel">
        {groups.length === 0 && (
          <p className="px-2 py-4 text-center text-sm text-ink-3">No matches.</p>
        )}
        {groups.map(([domain, perms]) => {
          const isOpen = open.has(domain) || q.trim().length > 0;
          const selectedCount = perms.filter((p) => selection.has(p.name)).length;
          return (
            <div key={domain} className="border-b border-line-soft last:border-0">
              <button
                type="button"
                onClick={() => {
                  const next = new Set(open);
                  if (next.has(domain)) next.delete(domain);
                  else next.add(domain);
                  setOpen(next);
                }}
                className="flex w-full items-center gap-2 px-2 py-1.5 text-left text-sm font-semibold hover:bg-panel-2"
              >
                {isOpen ? (
                  <ChevronDown className="size-4 text-ink-3" />
                ) : (
                  <ChevronRight className="size-4 text-ink-3" />
                )}
                <span className="font-mono">{domain}</span>
                {selectedCount > 0 && (
                  <StatusPill tone="brand" className="px-1.5 py-0 font-mono">
                    {selectedCount}
                  </StatusPill>
                )}
              </button>
              {isOpen && (
                <div className="px-2 pb-2">
                  {perms.map((it) => {
                    const sel = selection.get(it.name);
                    return (
                      <div key={it.name} className="py-1">
                        <label className="flex items-center gap-2 text-sm">
                          <input
                            type="checkbox"
                            disabled={disabled}
                            checked={!!sel}
                            className="size-3.5 accent-brand"
                            onChange={(e) => toggle(it, e.target.checked)}
                          />
                          <span className="font-mono text-xs">{it.name}</span>
                          {!it.bounds.national && (
                            <span className="text-xs text-ink-3">
                              (facility-scoped)
                            </span>
                          )}
                        </label>
                        {sel && (
                          <ScopeChips
                            bounds={it.bounds}
                            sel={sel}
                            facilities={facilities}
                            disabled={disabled}
                            onChange={(s) => setScope(it.name, s)}
                          />
                        )}
                      </div>
                    );
                  })}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
