import {useState} from "react";
import {ChevronDown} from "lucide-react";

import {useRouteCoverage} from "@/lib/fca";

/** Sidebar footer: share of live filed routes the nav engine resolves, with top unresolved tokens. */
export function CoveragePanel() {
  const coverage = useRouteCoverage();
  const [open, setOpen] = useState(false);
  const c = coverage.data;
  if (!c || c.pilots_with_route === 0) return null;
  const pct = c.resolved_pct;
  const tone = pct >= 90 ? "text-emerald-500" : pct >= 75 ? "text-amber-500" : "text-red-500";
  return (
    <div className="border-t px-4 py-2 text-xs">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        className="flex w-full items-center justify-between gap-2 text-left"
        title="Share of live filed routes the nav engine fully resolves"
      >
        <span className="text-muted-foreground">Route coverage</span>
        <span className="font-mono tabular-nums">
          <span className={tone}>{pct.toFixed(0)}%</span>
          <span className="text-muted-foreground">
            {" "}
            ({c.fully_resolved}/{c.pilots_with_route})
          </span>
          <ChevronDown className={`ml-1 inline size-3 transition-transform ${open ? "rotate-180" : ""}`} />
        </span>
      </button>
      {open && c.top_unresolved.length > 0 && (
        <div className="mt-1.5 max-h-40 overflow-y-auto rounded border bg-muted/20 p-1.5">
          <div className="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">
            Top unresolved tokens
          </div>
          <div className="grid grid-cols-2 gap-x-3 gap-y-0.5 font-mono">
            {c.top_unresolved.slice(0, 20).map((u) => (
              <div key={u.token} className="flex justify-between gap-2">
                <span className="truncate">{u.token}</span>
                <span className="shrink-0 text-muted-foreground">{u.count}</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
