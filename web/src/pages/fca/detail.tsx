import {useMemo, useState} from "react";
import {Button, ConfirmButton, Input, QueryState, Sheet, StatusPill, toneText} from "@ois/ui";

import {closestCenter, DndContext, type DragEndEvent, PointerSensor, useSensor, useSensors,} from "@dnd-kit/core";
import {arrayMove, SortableContext, useSortable, verticalListSortingStrategy,} from "@dnd-kit/sortable";
import {CSS} from "@dnd-kit/utilities";
import {GripVertical, PictureInPicture2, RotateCcw, Trash2, X} from "lucide-react";

import {DELAY_THRESHOLD_SEC, type Fca, type FcaFlight, fmtDelaySec, useClearRelease, useMarkRelease, useReorderFca,} from "@/lib/fca";
import {
  useExcludeFlight,
  useFlightExclusions,
  useRestoreFlight,
} from "@/lib/flight-exclusions";
import {flightStatus} from "@/lib/status";
import {hhmmZulu} from "@/lib/time";
import {Ladder} from "@/pages/fca/ladder";
import {can} from "@/lib/platform";
import {openPopout} from "@/lib/popout";

function fmtDelay(min: number): string {
  if (min <= 0) return "";
  const h = Math.floor(min / 60);
  const m = min % 60;
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}` : `${m}m`;
}

/** Short tag for who's being delayed: a released CFR, an airborne (needs vectors/speed), or ground. */
function delayTag(f: FcaFlight): string {
  if (f.released) return "CFR";
  return f.status === "airborne" ? "air" : "gnd";
}


function Strip({
  f,
  canEdit,
  canRemove,
  onRelease,
  onClear,
  onRemove,
}: {
  f: FcaFlight;
  canEdit: boolean;
  /** Whether the caller's `flow.fca.update` grant covers *this* FCA's ARTCC (#342). */
  canRemove: boolean;
  onRelease: (callsign: string, ready?: string) => void;
  onClear: (callsign: string) => void;
  /** Drop this flight as bogus (#342). */
  onRemove: (callsign: string) => void;
}) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } =
    useSortable({ id: f.callsign });
  const [hhmm, setHhmm] = useState("");
  const st = flightStatus(f.status);
  const canCfr = canEdit && f.status !== "airborne";

  const delayed = f.delay_sec >= DELAY_THRESHOLD_SEC;
  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : 1,
  };

  return (
    <li ref={setNodeRef} style={style} className="relative border-b border-line-soft bg-panel px-3 py-2 text-sm">
      <span
        aria-hidden="true"
        className="absolute inset-y-1 left-0 w-1 rounded-full"
        style={{ background: delayed ? "var(--danger)" : st.color }}
      />
      <div className="flex items-center justify-between gap-2">
        <span className="flex items-center gap-1.5">
          {canEdit && (
            <button
              type="button"
              aria-label={`Reorder ${f.callsign}`}
              className="cursor-grab text-ink-3 hover:text-ink"
              {...attributes}
              {...listeners}
            >
              <GripVertical className="size-3.5" />
            </button>
          )}
          <span className="w-4 text-right font-mono text-xs text-ink-3">{f.seq}</span>
          <StatusPill tone={st.tone} className="px-1.5 text-[10px] leading-4">
            {f.released ? "CFR" : st.label}
          </StatusPill>
          <span className="font-mono font-semibold">{f.callsign}</span>
          <span className="font-mono text-xs text-ink-3">{f.aircraft_type}</span>
          {canRemove && (
            <ConfirmButton
              size="icon"
              className="size-6"
              title="Remove bogus flight"
              aria-label={`Remove ${f.callsign} as a bogus flight`}
              warn={`Remove ${f.callsign} from the flow picture?`}
              onConfirm={() => onRemove(f.callsign)}
            >
              <Trash2 className="size-3.5" />
            </ConfirmButton>
          )}
        </span>
        <span className="text-right font-mono leading-tight">
          <span className={delayed ? "text-ink" : toneText[st.tone]}>
            {hhmmZulu(f.cross_time)}
          </span>
          <span
            className={`block text-xs ${delayed ? "text-danger" : "text-success"}`}
          >
            {delayed ? `${delayTag(f)} +${fmtDelaySec(f.delay_sec)}` : "on time"}
          </span>
        </span>
      </div>

      <div className="mt-0.5 flex items-center gap-2 pl-6 text-xs text-ink-3">
        <span className="font-mono">
          {f.dep}→{f.arr}
        </span>
        <span className="font-mono">{Math.round(f.distance_nm)}nm to line</span>
        {f.altitude > 0 && <span className="font-mono">FL{Math.round(f.altitude / 100)}</span>}
        {delayed && f.delay_nm > 0 && <span className="font-mono text-danger">+{f.delay_nm}nm</span>}
      </div>

      {canCfr && (
        <div className="mt-1.5 flex items-center gap-1.5 pl-6">
          {f.released ? (
            <>
              <StatusPill tone="good" className="font-mono">
                RLSD {hhmmZulu(f.edct)}
              </StatusPill>
              <button
                type="button"
                title="Clear release"
                onClick={() => onClear(f.callsign)}
                aria-label="Clear release"
                className="text-ink-3 transition-colors hover:text-danger"
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
                variant="outline"
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
        <div className="mt-1 ml-6 rounded-xs border border-dashed border-line bg-warning-soft px-2 py-1 font-mono text-[10px] leading-relaxed text-ink-2">
          <span>
            profile <span className="text-warning">{f.debug.profile}</span>
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
            <div className="text-danger">
              unresolved: {f.debug.unresolved.join(" ")}
            </div>
          )}
          {f.debug.taxi_estimate && (
            <div>
              push {f.debug.taxi_estimate.pushback_sec}s ({f.debug.taxi_estimate.pushback_tier}, n=
              {f.debug.taxi_estimate.pushback_samples}) · start-up {f.debug.taxi_estimate.startup_sec}s (
              {f.debug.taxi_estimate.startup_tier}, n={f.debug.taxi_estimate.startup_samples}) · taxi{" "}
              {f.debug.taxi_estimate.taxi_sec}s (
              {f.debug.taxi_estimate.taxi_tier}, n={f.debug.taxi_estimate.taxi_samples})
              {f.debug.taxi_estimate.gate && ` · gate ${f.debug.taxi_estimate.gate}`}
              {f.debug.taxi_estimate.runway && ` · rwy ${f.debug.taxi_estimate.runway}`}
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
  const exclusions = useFlightExclusions(fca.id);
  const excludeFlight = useExcludeFlight(fca.id);
  const restoreFlight = useRestoreFlight(fca.id);
  const removed = exclusions.data?.exclusions ?? [];
  // The server decides: `canEdit` only knows the caller holds `flow.fca.update` *somewhere*, and
  // these endpoints are ARTCC-scoped (#342). Offering the control on a facility the caller's grant
  // doesn't cover would just earn a 403.
  const canRemove = canEdit && exclusions.data?.editable === true;
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
    <Sheet
      className="h-full w-96 shrink-0 border-l border-line"
      onClose={onClose}
      initialFraction={0.45}
    >
      <div className="flex items-center gap-2 border-b border-line px-4 py-3">
        {/* The FCA's colour is user data. */}
        <span className="size-3 rounded-full" style={{ background: fca.color }} />
        <span className="font-mono font-semibold">{fca.name}</span>
        <StatusPill tone="neutral" className="font-mono">
          {fca.mode === "mit" ? `${fca.mit} MIT` : `${fca.rate}/hr`}
        </StatusPill>
        {fca.manual_seq && (
          <button
            type="button"
            title="Reset to automatic sequencing"
            onClick={() => reorder.mutate([])}
            className="ml-auto flex items-center gap-1 text-xs text-ink-3 hover:text-ink"
          >
            <RotateCcw className="size-3" /> manual
          </button>
        )}
        {/* Extra right padding on mobile so the header clears the sheet's close X. */}
        <span className="w-6 shrink-0 md:hidden" />
      </div>

      <div className="grid grid-cols-4 border-b border-line text-xs">
        {[
          { label: "Crossing", value: stats.total, cls: "text-ink" },
          { label: "Air", value: stats.air, cls: "text-flight-airborne" },
          { label: "Ground", value: stats.grd, cls: "text-flight-ground" },
          { label: "Σ delay", value: stats.delay > 0 ? fmtDelay(stats.delay) : "—", cls: stats.delay > 0 ? "text-danger" : "text-ink" },
        ].map((m) => (
          <div key={m.label} className="flex flex-col gap-0.5 px-4 py-2">
            <span className="text-ink-3">{m.label}</span>
            <span className={`font-mono text-base font-bold ${m.cls}`}>{m.value}</span>
          </div>
        ))}
      </div>

      <QueryState isLoading={!flights}>
        <div className="flex-1 overflow-y-auto">
          <div className="border-b border-line p-3">
            <div className="mb-2 flex items-center gap-2">
              <span className="text-xs font-semibold text-ink-2">Metering ladder · metered crossing</span>
              {/* Float the ladder over CRC/vATIS/charts. Desktop only (#349). */}
              {can("miniWindows") && (
                <Button
                  size="icon"
                  variant="ghost"
                  className="ml-auto size-6 text-ink-3 hover:text-ink"
                  title="Pop out into a floating window"
                  onClick={() =>
                    void openPopout({
                      id: `fca-${fca.id}`,
                      title: `${fca.name} · metering`,
                      route: `/popout/fca/${encodeURIComponent(fca.id)}`,
                    })
                  }
                >
                  <PictureInPicture2 className="size-3.5" />
                </Button>
              )}
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
                    canRemove={canRemove}
                    onRemove={(callsign) => excludeFlight.mutate({ callsign })}
                  />
                ))}
              </ul>
            </SortableContext>
          </DndContext>

          {canRemove && removed.length > 0 && (
            <div className="border-t border-line-soft px-3 py-2">
              <h3 className="mb-1.5 text-xs font-semibold text-ink-2">
                Removed flights
              </h3>
              <ul className="flex flex-col gap-1">
                {removed.map((x) => (
                  <li
                    key={x.callsign}
                    className="flex items-center justify-between gap-2 text-sm"
                  >
                    <span className="flex min-w-0 items-baseline gap-1.5">
                      <span className="font-mono font-semibold">
                        {x.callsign}
                      </span>
                      {x.created_by_name && (
                        <span className="truncate text-xs text-ink-3">
                          {x.created_by_name}
                        </span>
                      )}
                    </span>
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={() => restoreFlight.mutate(x.callsign)}
                    >
                      Restore
                    </Button>
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>
      </QueryState>
    </Sheet>
  );
}
