import {useMemo, useState} from "react";
import {Badge, Input, cn} from "@ois/ui";
import {ChevronDown, ChevronRight} from "lucide-react";

import type {ApiKeyPermission, ApiKeyPermissionInput, GrantablePermission} from "@/lib/api-keys";

/** Per-permission scope choice: grant nationally, or to the listed ARTCCs. */
export type ScopeSel = { national: boolean; artccs: string[] };
export type PermSelection = Map<string, ScopeSel>;

/** Flatten the picker's selection into the request's `(permission, artcc)` grants. */
export function buildPermissionInputs(selection: PermSelection): ApiKeyPermissionInput[] {
  const out: ApiKeyPermissionInput[] = [];
  for (const [permission, s] of selection) {
    if (s.national) out.push({ permission, artcc_id: null });
    else for (const artcc_id of s.artccs) out.push({ permission, artcc_id });
  }
  return out;
}

/** Seed a selection from a key's existing grants (for the edit flow). */
export function selectionFromPermissions(perms: ApiKeyPermission[]): PermSelection {
  const m: PermSelection = new Map();
  for (const p of perms) {
    const cur = m.get(p.permission) ?? { national: false, artccs: [] };
    if (p.artcc_id == null) cur.national = true;
    else if (!cur.artccs.includes(p.artcc_id)) cur.artccs.push(p.artcc_id);
    m.set(p.permission, cur);
  }
  return m;
}

/** Whether the selection is valid to submit (every scoped entry has at least one ARTCC). */
export function selectionIsValid(selection: PermSelection): boolean {
  for (const s of selection.values()) {
    if (!s.national && s.artccs.length === 0) return false;
  }
  return true;
}

function Chip({
  label,
  active,
  onClick,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "rounded border px-1.5 py-0.5 text-xs font-medium transition-colors",
        active
          ? "border-primary bg-primary text-primary-foreground"
          : "border-input text-muted-foreground hover:bg-accent",
      )}
    >
      {label}
    </button>
  );
}

/** The scope controls shown under a checked permission. */
function ScopeRow({
  grant,
  sel,
  facilities,
  onChange,
}: {
  grant: GrantablePermission;
  sel: ScopeSel;
  facilities: { id: string; name: string }[];
  onChange: (next: ScopeSel) => void;
}) {
  // A national holder may grant nationally OR narrow to any ARTCC; a scoped holder is limited to
  // the ARTCCs they hold the permission in.
  const options = grant.national ? facilities.map((f) => f.id) : grant.artccs;
  const toggleArtcc = (id: string) => {
    const has = sel.artccs.includes(id);
    onChange({
      national: false,
      artccs: has ? sel.artccs.filter((a) => a !== id) : [...sel.artccs, id],
    });
  };

  return (
    <div className="ml-6 mt-1 flex flex-wrap items-center gap-1.5">
      {grant.national && (
        <Chip
          label="National"
          active={sel.national}
          onClick={() => onChange({ national: true, artccs: [] })}
        />
      )}
      {grant.national && <span className="text-xs text-muted-foreground">or</span>}
      {options.map((id) => (
        <Chip key={id} label={id} active={!sel.national && sel.artccs.includes(id)} onClick={() => toggleArtcc(id)} />
      ))}
      {!sel.national && sel.artccs.length === 0 && (
        <span className="text-xs text-destructive">pick at least one ARTCC</span>
      )}
    </div>
  );
}

/**
 * Flat permission picker for API keys. Renders the caller's grantable permissions grouped by domain;
 * each checked permission gets a scope control (National or specific ARTCCs) bounded by what the
 * caller can delegate.
 */
export function PermissionPicker({
  grantable,
  facilities,
  selection,
  onChange,
}: {
  grantable: GrantablePermission[];
  facilities: { id: string; name: string }[];
  selection: PermSelection;
  onChange: (next: PermSelection) => void;
}) {
  const [q, setQ] = useState("");
  const [open, setOpen] = useState<Set<string>>(() => new Set());

  const groups = useMemo(() => {
    const needle = q.trim().toLowerCase();
    const byDomain = new Map<string, GrantablePermission[]>();
    for (const g of grantable) {
      if (needle && !g.permission.toLowerCase().includes(needle)) continue;
      const domain = g.permission.split(".")[0];
      const list = byDomain.get(domain) ?? [];
      list.push(g);
      byDomain.set(domain, list);
    }
    return [...byDomain.entries()].sort(([a], [b]) => a.localeCompare(b));
  }, [grantable, q]);

  const toggle = (grant: GrantablePermission, on: boolean) => {
    const next = new Map(selection);
    if (on) {
      next.set(
        grant.permission,
        grant.national ? { national: true, artccs: [] } : { national: false, artccs: [...grant.artccs] },
      );
    } else {
      next.delete(grant.permission);
    }
    onChange(next);
  };

  const setScope = (permission: string, s: ScopeSel) => {
    const next = new Map(selection);
    next.set(permission, s);
    onChange(next);
  };

  if (grantable.length === 0) {
    return (
      <p className="rounded-md border border-dashed p-4 text-center text-sm text-muted-foreground">
        You don&apos;t hold any permissions that can be delegated to a key.
      </p>
    );
  }

  return (
    <div className="flex flex-col gap-2">
      <Input
        value={q}
        onChange={(e) => setQ(e.target.value)}
        placeholder="Filter permissions…"
        className="h-8"
      />
      <div className="max-h-80 overflow-y-auto rounded-md border">
        {groups.map(([domain, perms]) => {
          const isOpen = open.has(domain) || q.trim().length > 0;
          const selectedCount = perms.filter((p) => selection.has(p.permission)).length;
          return (
            <div key={domain} className="border-b last:border-0">
              <button
                type="button"
                onClick={() => {
                  const next = new Set(open);
                  if (next.has(domain)) next.delete(domain);
                  else next.add(domain);
                  setOpen(next);
                }}
                className="flex w-full items-center gap-2 px-2 py-1.5 text-left text-sm font-medium hover:bg-accent"
              >
                {isOpen ? <ChevronDown className="size-4" /> : <ChevronRight className="size-4" />}
                <span className="font-mono">{domain}</span>
                {selectedCount > 0 && (
                  <Badge variant="secondary" className="text-[10px]">
                    {selectedCount}
                  </Badge>
                )}
              </button>
              {isOpen && (
                <div className="px-2 pb-2">
                  {perms.map((grant) => {
                    const sel = selection.get(grant.permission);
                    return (
                      <div key={grant.permission} className="py-1">
                        <label className="flex items-center gap-2 text-sm">
                          <input
                            type="checkbox"
                            checked={!!sel}
                            onChange={(e) => toggle(grant, e.target.checked)}
                          />
                          <span className="font-mono text-xs">{grant.permission}</span>
                          {!grant.national && (
                            <span className="text-[10px] text-muted-foreground">(facility-scoped)</span>
                          )}
                        </label>
                        {sel && (
                          <ScopeRow
                            grant={grant}
                            sel={sel}
                            facilities={facilities}
                            onChange={(s) => setScope(grant.permission, s)}
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
