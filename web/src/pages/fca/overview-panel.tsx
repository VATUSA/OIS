import {useState} from "react";
import {Badge} from "@ois/ui";
import {ChevronDown, ChevronRight} from "lucide-react";

import {BottomSheet} from "@/components/bottom-sheet";
import {type Fca, type FcaFlight} from "@/lib/fca";
import {hhmmZulu} from "@/lib/time";

type Filter = "all" | "air" | "cfr";

const STATUS = {
  airborne: { label: "AIR", color: "#22c55e", text: "text-emerald-500" },
  ground: { label: "GND", color: "#f59e0b", text: "text-amber-500" },
  proposed: { label: "PROP", color: "#38bdf8", text: "text-sky-400" },
} as const;
function statusOf(s: string) {
  return STATUS[s as keyof typeof STATUS] ?? STATUS.ground;
}

const modeLabel = (fca: Fca) => (fca.mode === "mit" ? `${fca.mit} MIT` : `${fca.rate}/hr`);

function matchesFilter(f: FcaFlight, filter: Filter): boolean {
  if (filter === "air") return f.status === "airborne";
  if (filter === "cfr") return f.released;
  return true;
}

export interface OverviewGroup {
  fca: Fca;
  flights: FcaFlight[] | undefined;
}

/**
 * The ARTCC overview's right-hand "strips" panel: every active FCA in the selected ARTCC stacked as a
 * compact, collapsible card showing its crossing traffic. Read-only — this is the public advisories
 * view; controllers manage releases from the per-FCA tool. Mirrors vatflow's artcc-dashboard strips.
 */
export function FcaOverviewPanel({
  artcc,
  groups,
  onFocusFlight,
  onClose,
}: {
  artcc: string;
  groups: OverviewGroup[];
  onFocusFlight?: (callsign: string) => void;
  onClose?: () => void;
}) {
  const [filter, setFilter] = useState<Filter>("all");
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const toggle = (id: string) =>
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  return (
    <BottomSheet desktopClassName="h-full w-96 shrink-0 border-l" onClose={onClose} initialFraction={0.5}>
      <div className="flex items-center gap-2 border-b px-4 py-3">
        <span className="font-mono text-sm font-semibold uppercase tracking-wide">Strips · {artcc}</span>
        <div className="ml-auto flex items-center gap-1">
          {(["all", "air", "cfr"] as Filter[]).map((k) => (
            <button
              key={k}
              type="button"
              onClick={() => setFilter(k)}
              className={
                "rounded px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide transition-colors " +
                (filter === k
                  ? "bg-primary text-primary-foreground"
                  : "text-muted-foreground hover:text-foreground")
              }
            >
              {k === "cfr" ? "CFR" : k}
            </button>
          ))}
        </div>
        {/* Clear the mobile sheet's close X. */}
        <span className="w-6 shrink-0 md:hidden" />
      </div>

      <div className="flex-1 overflow-y-auto">
        {groups.length === 0 ? (
          <p className="p-4 text-sm text-muted-foreground">No active FCAs for {artcc}.</p>
        ) : (
          groups.map(({ fca, flights }) => {
            const list = (flights ?? []).filter((f) => f.status !== "proposed");
            const air = list.filter((f) => f.status === "airborne").length;
            const cfr = list.filter((f) => f.released).length;
            const shown = list.filter((f) => matchesFilter(f, filter));
            const isCollapsed = collapsed.has(fca.id);
            return (
              <div key={fca.id} className="border-b">
                <button
                  type="button"
                  onClick={() => toggle(fca.id)}
                  className="flex w-full items-center gap-2 px-3 py-2 text-left hover:bg-accent/30"
                >
                  {isCollapsed ? (
                    <ChevronRight className="size-3.5 shrink-0 text-muted-foreground" />
                  ) : (
                    <ChevronDown className="size-3.5 shrink-0 text-muted-foreground" />
                  )}
                  <span className="size-2.5 shrink-0 rounded-sm" style={{ background: fca.color }} />
                  <span className="truncate font-mono text-sm font-semibold">{fca.name}</span>
                  <Badge variant="secondary">{modeLabel(fca)}</Badge>
                  {fca.manual_seq && (
                    <span className="rounded border px-1 text-[10px] font-semibold text-muted-foreground">
                      MANUAL
                    </span>
                  )}
                  <span className="ml-auto shrink-0 tabular-nums text-xs text-muted-foreground">
                    {air} air · {cfr} CFR
                  </span>
                </button>
                {!isCollapsed &&
                  (flights == null ? (
                    <p className="px-3 pb-2 text-xs text-muted-foreground">Loading…</p>
                  ) : shown.length === 0 ? (
                    <p className="px-3 pb-3 text-center text-xs text-muted-foreground">
                      No {filter === "air" ? "airborne " : filter === "cfr" ? "CFR " : ""}traffic crossing
                      this FCA.
                    </p>
                  ) : (
                    <ul className="pb-1">
                      {shown.map((f) => (
                        <CompactStrip key={f.callsign} f={f} onClick={() => onFocusFlight?.(f.callsign)} />
                      ))}
                    </ul>
                  ))}
              </div>
            );
          })
        )}
      </div>
    </BottomSheet>
  );
}

/** A single read-only crossing strip — click to locate the aircraft on the map. */
function CompactStrip({ f, onClick }: { f: FcaFlight; onClick?: () => void }) {
  const st = statusOf(f.status);
  const delayed = f.delay_sec >= 30;
  return (
    <li>
      <button
        type="button"
        onClick={onClick}
        className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs hover:bg-accent/30"
        style={{ borderLeft: `3px solid ${delayed ? "#ef4444" : st.color}` }}
      >
        <span className="w-4 shrink-0 text-right tabular-nums text-muted-foreground">{f.seq}</span>
        <span
          className="shrink-0 rounded px-1 text-[10px] font-semibold"
          style={{ color: st.color, border: `1px solid ${st.color}` }}
        >
          {f.released ? "CFR" : st.label}
        </span>
        <span className="font-mono font-semibold">{f.callsign}</span>
        <span className="truncate font-mono text-muted-foreground">
          {f.dep}→{f.arr}
        </span>
        <span className="ml-auto shrink-0 text-right font-mono leading-tight">
          <span className={delayed ? "text-foreground" : st.text}>{hhmmZulu(f.cross_time)}</span>
          <span className={`block text-[10px] ${delayed ? "text-destructive" : "text-emerald-500"}`}>
            {delayed ? `+${Math.round(f.delay_sec / 60)}m` : "on time"}
          </span>
        </span>
      </button>
    </li>
  );
}
