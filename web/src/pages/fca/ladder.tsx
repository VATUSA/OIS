import {type FcaFlight} from "@/lib/fca";
import {flightStatus} from "@/lib/status";
import {hhmmZulu} from "@/lib/time";
import {ArrivalLadder} from "@/components/ladder/ArrivalLadder";

/**
 * The FCA metering ladder — every crossing plotted by its metered time, now at the bottom.
 *
 * Its own file because it is rendered from two places: the FCA detail panel, and a pop-out
 * mini-window that floats it over other applications (#349). It takes only data, so it works in
 * either — the window it happens to be in is not its concern.
 */

const LADDER_WIN = 60;
const LADDER_CH = 7; // ≈ px per monospace/tabular char at text-xs

/** Estimated rendered pixel width of one metering tag — connector + pill padding/border/gaps +
 * text (seq + callsign + "HH:MMz"-ish time). Mirrors `airport.tsx`'s `stripW`. */
function measureTagWidth(f: FcaFlight): number {
  const chars = String(f.seq).length + f.callsign.length + 5;
  return 12 /* connector tick */ + 42 /* pill padding + border + dot + gaps */ + chars * LADDER_CH;
}

function minutesUntil(iso: string | null | undefined, now: number): number | null {
  if (!iso) return null;
  const t = new Date(iso).getTime();
  return Number.isNaN(t) ? null : (t - now) / 60000;
}

export function Ladder({ flights, now }: { flights: FcaFlight[]; now: number }) {
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
      connectorColor={(f) => flightStatus(f.status).color}
      renderTag={(f) => {
        const st = flightStatus(f.status);
        return (
          <span className="flex items-center gap-1.5 rounded-xs border border-line bg-panel-2 py-0.5 pl-1.5 pr-2 text-xs">
            <span className="size-1.5 shrink-0 rounded-full" style={{ background: st.color }} />
            <span className="font-mono text-ink-3">{f.seq}</span>
            <span className="font-mono font-semibold">{f.callsign}</span>
            <span className="font-mono text-ink-2">{hhmmZulu(f.cross_time)}</span>
          </span>
        );
      }}
    />
  );
}
