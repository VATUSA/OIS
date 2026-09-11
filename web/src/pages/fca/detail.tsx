import {useMemo, useState} from "react";
import {Badge, Button, Input} from "@ois/ui";

import {BottomSheet} from "@/components/bottom-sheet";
import {closestCenter, DndContext, type DragEndEvent, PointerSensor, useSensor, useSensors,} from "@dnd-kit/core";
import {arrayMove, SortableContext, useSortable, verticalListSortingStrategy,} from "@dnd-kit/sortable";
import {CSS} from "@dnd-kit/utilities";
import {GripVertical, RotateCcw, X} from "lucide-react";

import {type Fca, type FcaFlight, useClearRelease, useMarkRelease, useReorderFca,} from "@/lib/fca";
import {hhmmZulu} from "@/lib/time";
import {ArrivalLadder} from "@/components/ladder/ArrivalLadder";

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

/** A metered delay of ~1 min or more is worth flagging (below that is rounding noise). */
const DELAY_THRESHOLD_SEC = 30;

/** Delay as `M:SS` (e.g. 1268 → "21:08"). */
function fmtDelaySec(sec: number): string {
  const m = Math.floor(sec / 60);
  const s = sec % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

/** Short tag for who's being delayed: a released CFR, an airborne (needs vectors/speed), or ground. */
function delayTag(f: FcaFlight): string {
  if (f.released) return "CFR";
  return f.status === "airborne" ? "air" : "gnd";
}

const LADDER_WIN = 60;
const LADDER_CH = 7; // ≈ px per monospace/tabular char at text-xs

/** Estimated rendered pixel width of one metering tag — connector + pill padding/border/gaps +
 * text (seq + callsign + "HH:MMz"-ish time). Mirrors `airport.tsx`'s `stripW`. */
function measureTagWidth(f: FcaFlight): number {
  const chars = String(f.seq).length + f.callsign.length + 5;
  return 12 /* connector tick */ + 30 /* pill padding + border + gaps */ + chars * LADDER_CH;
}

/** Metering ladder — plots each flight by its metered crossing time (now at bottom). */
function Ladder({ flights, now }: { flights: FcaFlight[]; now: number }) {
  const items = flights
    .map((f) => ({ f, min: minutesUntil(f.cross_time, now) }))
    // `min` is non-null only when `cross_time` was itself a valid, non-empty timestamp.
    .filter((x): x is { f: FcaFlight; min: number } => x.min != null && !!x.f.cross_time)
    .map((x) => ({ key: x.f.callsign, min: x.min, time: x.f.cross_time as string, data: x.f }));

  return (
    <ArrivalLadder
      items={items}
      now={now}
      win={LADDER_WIN}
      pxPerMin={5}
      gutter={54}
      step={10}
      rowGap={22}
      minGap={11}
      pad={6}
      minWidth={260}
      emptyMessage={`No crossings in the next ${LADDER_WIN} min.`}
      measureTagWidth={measureTagWidth}
      connectorColor={(f) => statusOf(f.status).color}
      renderTag={(f) => {
        const st = statusOf(f.status);
        return (
          <span
            className="flex items-center gap-1.5 rounded border border-border/70 bg-muted/40 py-0.5 pl-1.5 pr-2 text-xs"
            style={{ borderLeftWidth: 3, borderLeftColor: st.color }}
          >
            <span className="tabular-nums text-muted-foreground">{f.seq}</span>
            <span className="font-mono font-medium">{f.callsign}</span>
            <span className="font-mono text-muted-foreground">{hhmmZulu(f.cross_time)}</span>
          </span>
        );
      }}
    />
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

  const delayed = f.delay_sec >= DELAY_THRESHOLD_SEC;
  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : 1,
    borderLeftWidth: 3,
    borderLeftStyle: "solid" as const,
    borderLeftColor: delayed ? "#ef4444" : st.color,
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
        <span className="text-right font-mono leading-tight">
          <span className={delayed ? "text-foreground" : st.text}>
            {hhmmZulu(f.cross_time)}
          </span>
          <span
            className={`block text-xs ${delayed ? "text-destructive" : "text-emerald-500"}`}
          >
            {delayed ? `${delayTag(f)} +${fmtDelaySec(f.delay_sec)}` : "on time"}
          </span>
        </span>
      </div>

      <div className="mt-0.5 flex items-center gap-2 pl-6 text-xs text-muted-foreground">
        <span className="font-mono">
          {f.dep}→{f.arr}
        </span>
        <span>{Math.round(f.distance_nm)}nm to line</span>
        {f.altitude > 0 && <span>FL{Math.round(f.altitude / 100)}</span>}
        {delayed && f.delay_nm > 0 && (
          <span className="text-destructive">+{f.delay_nm}nm</span>
        )}
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

      {f.debug && (
        <div className="mt-1 ml-6 rounded border border-dashed border-amber-500/40 bg-amber-500/5 px-2 py-1 font-mono text-[10px] leading-relaxed text-muted-foreground">
          <span>
            profile <span className="text-amber-600 dark:text-amber-400">{f.debug.profile}</span>
          </span>
          {" · "}
          <span>{Math.round(f.debug.cruise_tas)}kt TAS @ FL{Math.round(f.debug.cruise_alt / 100)}</span>
          {f.debug.headwind != null && (
            <>
              {" · "}
              <span>
                {f.debug.headwind >= 0 ? "HW" : "TW"} {Math.abs(Math.round(f.debug.headwind))}kt
              </span>
            </>
          )}
          {f.debug.unresolved.length > 0 && (
            <div className="text-destructive">
              unresolved: {f.debug.unresolved.join(" ")}
            </div>
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
  // Proposed = prefiled flights whose pilot hasn't connected yet; they have no real crossing time, so
  // this panel shows only connected (airborne/ground) traffic.
  const list = (flights ?? []).filter((f) => f.status !== "proposed");
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
    <BottomSheet
      desktopClassName="h-full w-96 shrink-0 border-l"
      onClose={onClose}
      initialFraction={0.45}
    >
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
        {/* Extra right padding on mobile so the header clears the sheet's close X. */}
        <span className="w-6 shrink-0 md:hidden" />
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
    </BottomSheet>
  );
}
