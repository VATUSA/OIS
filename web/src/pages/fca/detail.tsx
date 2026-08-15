import {useMemo, useState} from "react";
import {Badge, Button, Input} from "@ois/ui";
import {closestCenter, DndContext, type DragEndEvent, PointerSensor, useSensor, useSensors,} from "@dnd-kit/core";
import {arrayMove, SortableContext, useSortable, verticalListSortingStrategy,} from "@dnd-kit/sortable";
import {CSS} from "@dnd-kit/utilities";
import {GripVertical, RotateCcw, X} from "lucide-react";

import {type Fca, type FcaFlight, useClearRelease, useMarkRelease, useReorderFca,} from "@/lib/fca";
import {hhmmZulu} from "@/lib/time";

const STATUS = {
  airborne: { label: "AIR", color: "#22c55e", text: "text-emerald-500" },
  ground: { label: "GND", color: "#f59e0b", text: "text-amber-500" },
  proposed: { label: "PROP", color: "#38bdf8", text: "text-sky-400" },
} as const;

function statusOf(s: string) {
  return STATUS[s as keyof typeof STATUS] ?? STATUS.ground;
}

function minutesUntil(iso: string | null | undefined, now: number): number | null {
  if (!iso) return null;
  const t = new Date(iso).getTime();
  return Number.isNaN(t) ? null : (t - now) / 60000;
}

function fmtDelay(min: number): string {
  if (min <= 0) return "";
  const h = Math.floor(min / 60);
  const m = min % 60;
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}` : `${m}m`;
}

/** Metering ladder — plots each flight by its metered crossing time (now at bottom). */
function Ladder({ flights, now }: { flights: FcaFlight[]; now: number }) {
  const WIN = 60;
  const PX = 5;
  const ROW = 22;
  const GUTTER = 54;
  const H = WIN * PX;
  const yOf = (min: number) => H - (Math.max(0, Math.min(min, WIN)) / WIN) * H;

  const items = flights
    .map((f) => ({ f, min: minutesUntil(f.cross_time, now) }))
    .filter((x): x is { f: FcaFlight; min: number } => x.min != null)
    .filter((x) => x.min >= -1 && x.min <= WIN)
    .sort((a, b) => a.min - b.min);

  let lastY = H + ROW;
  const placed = items.map(({ f, min }) => {
    const y = Math.min(yOf(min), lastY - ROW);
    lastY = y;
    return { f, min, y };
  });
  const topY = placed.length ? placed[placed.length - 1].y : H;
  const shift = topY < 6 ? 6 - topY : 0;
  const contentH = H + shift + 10;

  const grid = [];
  for (let k = 0; k <= WIN / 10; k++) {
    const y = yOf(k * 10) + shift;
    grid.push(
      <div key={k}>
        <div
          className="absolute border-t border-border/40"
          style={{ top: y, left: GUTTER, right: 0 }}
        />
        <span
          className="absolute font-mono text-[10px] text-muted-foreground"
          style={{ top: y - 6, left: 0, width: GUTTER - 8, textAlign: "right" }}
        >
          {hhmmZulu(new Date(now + k * 10 * 60000).toISOString())}
        </span>
      </div>,
    );
  }

  if (placed.length === 0) {
    return (
      <p className="py-6 text-center text-xs text-muted-foreground">
        No crossings in the next {WIN} min.
      </p>
    );
  }

  return (
    <div className="relative" style={{ height: contentH, minWidth: 260 }}>
      <div
        className="absolute top-0 bottom-0 border-l border-border/60"
        style={{ left: GUTTER }}
      />
      {grid}
      <div
        className="absolute border-t-2 border-primary"
        style={{ top: yOf(0) + shift, left: GUTTER, right: 0 }}
      >
        <span
          className="absolute -top-2 text-[10px] font-semibold text-primary"
          style={{ left: 0, width: GUTTER - 8, textAlign: "right" }}
        >
          NOW
        </span>
      </div>
      {placed.map(({ f, y }) => {
        const st = statusOf(f.status);
        return (
          <div
            key={f.callsign}
            className="absolute flex items-center"
            style={{ top: y + shift - 10, left: GUTTER }}
          >
            <span className="h-0.5 w-3 shrink-0" style={{ background: st.color }} />
            <span
              className="flex items-center gap-1.5 rounded border border-border/70 bg-muted/40 py-0.5 pl-1.5 pr-2 text-xs"
              style={{ borderLeftWidth: 3, borderLeftColor: st.color }}
            >
              <span className="tabular-nums text-muted-foreground">{f.seq}</span>
              <span className="font-mono font-medium">{f.callsign}</span>
              <span className="font-mono text-muted-foreground">
                {hhmmZulu(f.cross_time)}
              </span>
            </span>
          </div>
        );
      })}
    </div>
  );
}

function Strip({
  f,
  canEdit,
  onRelease,
  onClear,
}: {
  f: FcaFlight;
  canEdit: boolean;
  onRelease: (callsign: string, ready?: string) => void;
  onClear: (callsign: string) => void;
}) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } =
    useSortable({ id: f.callsign });
  const [hhmm, setHhmm] = useState("");
  const st = statusOf(f.status);
  const canCfr = canEdit && f.status !== "airborne";

  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : 1,
  };

  return (
    <li ref={setNodeRef} style={style} className="border-b bg-background px-3 py-2 text-sm">
      <div className="flex items-center justify-between gap-2">
        <span className="flex items-center gap-1.5">
          {canEdit && (
            <button
              type="button"
              className="cursor-grab text-muted-foreground/60 hover:text-foreground"
              {...attributes}
              {...listeners}
            >
              <GripVertical className="size-3.5" />
            </button>
          )}
          <span className="w-4 text-right tabular-nums text-muted-foreground">
            {f.seq}
          </span>
          <span
            className="rounded px-1 text-[10px] font-semibold"
            style={{ color: st.color, border: `1px solid ${st.color}` }}
          >
            {f.released ? "CFR" : st.label}
          </span>
          <span className="font-mono font-semibold">{f.callsign}</span>
          <span className="text-xs text-muted-foreground">{f.aircraft_type}</span>
        </span>
        <span className="text-right font-mono">
          <span className={st.text}>{hhmmZulu(f.cross_time)}</span>
          {f.delay_min > 0 && (
            <span className="ml-1.5 text-xs text-destructive">+{f.delay_min}m</span>
          )}
        </span>
      </div>

      <div className="mt-0.5 flex items-center gap-2 pl-6 text-xs text-muted-foreground">
        <span className="font-mono">
          {f.dep}→{f.arr}
        </span>
        <span>{Math.round(f.distance_nm)}nm to line</span>
        {f.altitude > 0 && <span>FL{Math.round(f.altitude / 100)}</span>}
      </div>

      {canCfr && (
        <div className="mt-1.5 flex items-center gap-1.5 pl-6">
          {f.released ? (
            <>
              <Badge variant="success">RLSD {hhmmZulu(f.edct)}z</Badge>
              <button
                type="button"
                title="Clear release"
                onClick={() => onClear(f.callsign)}
                className="text-muted-foreground transition-colors hover:text-destructive"
              >
                <X className="size-3.5" />
              </button>
            </>
          ) : (
            <>
              <Input
                className="h-7 w-20 font-mono"
                placeholder="HHMMz"
                maxLength={5}
                value={hhmm}
                onChange={(e) => setHhmm(e.target.value)}
              />
              <Button
                size="sm"
                variant="secondary"
                disabled={hhmm.trim().length < 4}
                onClick={() => onRelease(f.callsign, hhmm.trim())}
              >
                SET
              </Button>
              <Button size="sm" onClick={() => onRelease(f.callsign)}>
                RDY
              </Button>
            </>
          )}
        </div>
      )}
    </li>
  );
}

export function FcaDetail({
  fca,
  flights,
  canEdit,
  onClose,
}: {
  fca: Fca;
  flights: FcaFlight[] | undefined;
  canEdit: boolean;
  onClose?: () => void;
}) {
  const now = Date.now();
  const list = flights ?? [];
  const markRelease = useMarkRelease(fca.id);
  const clearRelease = useClearRelease(fca.id);
  const reorder = useReorderFca(fca.id);
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
  );

  const stats = useMemo(() => {
    const air = list.filter((f) => f.status === "airborne").length;
    const grd = list.length - air;
    const delay = list.reduce((s, f) => s + Math.max(0, f.delay_min), 0);
    return { air, grd, delay, total: list.length };
  }, [list]);

  function onDragEnd(e: DragEndEvent) {
    const { active, over } = e;
    if (!over || active.id === over.id) return;
    const ids = list.map((f) => f.callsign);
    const from = ids.indexOf(String(active.id));
    const to = ids.indexOf(String(over.id));
    if (from < 0 || to < 0) return;
    reorder.mutate(arrayMove(ids, from, to));
  }

  return (
    <div className="flex h-full w-96 shrink-0 flex-col border-l bg-background max-md:absolute max-md:inset-y-0 max-md:right-0 max-md:z-[700] max-md:w-[92%] max-md:max-w-sm max-md:shadow-2xl">
      <div className="flex items-center gap-2 border-b px-4 py-3">
        <span className="size-3 rounded-full" style={{ background: fca.color }} />
        <span className="font-mono font-semibold">{fca.name}</span>
        <Badge variant="secondary">
          {fca.mode === "mit" ? `${fca.mit} MIT` : `${fca.rate}/hr`}
        </Badge>
        {fca.manual_seq && (
          <button
            type="button"
            title="Reset to automatic sequencing"
            onClick={() => reorder.mutate([])}
            className="ml-auto flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
          >
            <RotateCcw className="size-3" /> manual
          </button>
        )}
        {onClose && (
          <button
            type="button"
            aria-label="Close"
            onClick={onClose}
            className="ml-auto text-muted-foreground hover:text-foreground md:hidden"
          >
            <X className="size-4" />
          </button>
        )}
      </div>

      <div className="flex flex-wrap gap-x-4 gap-y-1 border-b px-4 py-2 text-xs">
        <span>
          <span className="text-muted-foreground">crossing </span>
          <span className="font-semibold tabular-nums">{stats.total}</span>
        </span>
        <span>
          <span className="text-muted-foreground">air </span>
          <span className="font-semibold tabular-nums text-emerald-500">
            {stats.air}
          </span>
        </span>
        <span>
          <span className="text-muted-foreground">ground </span>
          <span className="font-semibold tabular-nums text-amber-500">
            {stats.grd}
          </span>
        </span>
        <span>
          <span className="text-muted-foreground">Σdelay </span>
          <span
            className={
              "font-semibold tabular-nums " +
              (stats.delay > 0 ? "text-destructive" : "")
            }
          >
            {stats.delay > 0 ? fmtDelay(stats.delay) : "—"}
          </span>
        </span>
      </div>

      {!flights ? (
        <p className="p-4 text-sm text-muted-foreground">Loading…</p>
      ) : (
        <div className="flex-1 overflow-y-auto">
          <div className="border-b p-3">
            <div className="mb-2 text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
              Metering ladder · metered crossing
            </div>
            <Ladder flights={list} now={now} />
          </div>

          <DndContext
            sensors={sensors}
            collisionDetection={closestCenter}
            onDragEnd={onDragEnd}
          >
            <SortableContext
              items={list.map((f) => f.callsign)}
              strategy={verticalListSortingStrategy}
            >
              <ul>
                {list.map((f) => (
                  <Strip
                    key={f.callsign}
                    f={f}
                    canEdit={canEdit}
                    onRelease={(callsign, ready) =>
                      markRelease.mutate({ callsign, ready })
                    }
                    onClear={(callsign) => clearRelease.mutate(callsign)}
                  />
                ))}
              </ul>
            </SortableContext>
          </DndContext>
        </div>
      )}
    </div>
  );
}
