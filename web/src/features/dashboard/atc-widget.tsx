import {useAtc, type AtcBoard, type AtcPosition} from "@/lib/fca";
import {facilityAirports, facilityKindLabel, useFacilityDirectory} from "@/lib/facilities";
import {onlineFor, ratingLabel} from "@/lib/atc-format";
import {hhmmZulu} from "@/lib/time";

import type {AtcWidget, FacilityRef} from "./types";
import {useReportWidgetStatus} from "./widget-status";

/** ATC position kind → its domain-token badge (tinted fill + same-hue text). */
const KIND_BADGE: Record<string, string> = {
  DEL: "bg-atc-del/15 text-atc-del",
  GND: "bg-atc-gnd/15 text-atc-gnd",
  TWR: "bg-atc-twr/15 text-atc-twr",
  APP: "bg-atc-app/15 text-atc-app",
  CTR: "bg-atc-ctr/15 text-atc-ctr",
  ATIS: "bg-atc-atis/15 text-atc-atis",
};

const posName = (p: AtcPosition) =>
  p.kind === "ATIS" ? `ATIS${p.atis_code ? " " + p.atis_code : ""}` : p.callsign;

function PositionRow({ p }: { p: AtcPosition }) {
  const rating = ratingLabel(p.rating);
  const online = onlineFor(p.logon_time);
  return (
    <div className="flex items-baseline gap-2 py-1 text-sm">
      <span className={"shrink-0 rounded-xs px-1 font-mono text-[10px] font-bold " + (KIND_BADGE[p.kind] ?? "bg-chip text-ink-2")}>
        {p.kind}
      </span>
      <span className="font-mono font-semibold">{posName(p)}</span>
      <span className="font-mono text-xs text-ink-3">{p.frequency}</span>
      {p.name && (
        <span className="ml-auto truncate text-xs text-ink-3">
          {p.name}
          {rating && ` · ${rating}`}
          {online && ` · ${online}`}
        </span>
      )}
    </div>
  );
}

/** One labelled block of positions. `mono` renders the heading as an identifier (an ICAO / facility id). */
type PositionGroup = { key: string; label: string; mono: boolean; positions: AtcPosition[] };

const withPositions = (g: PositionGroup): boolean => g.positions.length > 0;

/**
 * Every online position in the country, grouped: centers, then approach areas, then airport stacks.
 *
 * The board is already national — `useAtc` makes one `GET /api/v1/flow/atc` — so this is the
 * unfiltered view of what the facility branch narrows.
 */
function nationalGroups(board: AtcBoard | undefined): PositionGroup[] {
  const byId = <T extends { id: string }>(xs: T[]) =>
    xs.slice().sort((a, b) => a.id.localeCompare(b.id));
  return [
    ...byId(board?.centers ?? []).map((c) => ({
      key: `ctr:${c.id}`,
      label: c.id,
      mono: true,
      positions: c.positions,
    })),
    ...byId(board?.tracons ?? []).map((t) => ({
      key: `app:${t.id}`,
      label: t.id,
      mono: true,
      positions: t.positions,
    })),
    ...(board?.airports ?? [])
      .slice()
      .sort((a, b) => a.icao.localeCompare(b.icao))
      .map((a) => ({ key: `apt:${a.icao}`, label: a.icao, mono: true, positions: a.positions })),
  ].filter(withPositions);
}

/** Online ATC for one facility: its center/approach positions + its airports' ground stacks. */
function facilityGroups(
  board: AtcBoard | undefined,
  facility: FacilityRef,
  memberIcaos: string[],
): PositionGroup[] {
  const own =
    facility.kind === "artcc"
      ? (board?.centers.find((c) => c.id === facility.id)?.positions ?? [])
      : (board?.tracons.find((t) => t.id === facility.id)?.positions ?? []);
  return [
    {
      key: "own",
      label: facility.kind === "artcc" ? "Center" : "Approach / Departure",
      mono: false,
      positions: own,
    },
    ...(board?.airports ?? [])
      .filter((a) => memberIcaos.includes(a.icao))
      .sort((a, b) => a.icao.localeCompare(b.icao))
      .map((a) => ({ key: `apt:${a.icao}`, label: a.icao, mono: true, positions: a.positions })),
  ].filter(withPositions);
}

/** Online ATC for a facility, or for the whole NAS when the widget is scoped nationally. */
export function AtcWidgetView({ widget }: { widget: AtcWidget }) {
  const { facility } = widget;
  const national = facility.kind === "national";
  const { data: board, isLoading, isFetching, dataUpdatedAt, refetch } = useAtc(true);
  // Only the facility branch resolves member airports; the national view shows every airport as-is.
  const dir = useFacilityDirectory();
  useReportWidgetStatus(isFetching, dataUpdatedAt, refetch);

  const groups = national
    ? nationalGroups(board)
    : facilityGroups(board, facility, facilityAirports(dir.data, facility.id));

  return (
    <div className="flex h-full flex-col overflow-y-auto p-3 text-ink">
      {isLoading && !board ? (
        <p className="text-sm text-ink-3">Loading…</p>
      ) : groups.length === 0 ? (
        <p className="text-sm text-ink-3">
          {national
            ? "No online ATC anywhere in the NAS."
            : `No online ATC for ${facility.id} (${facilityKindLabel(facility.kind)}).`}
        </p>
      ) : (
        <div className="flex flex-col gap-3">
          {groups.map((g) => (
            <div key={g.key}>
              <div
                className={
                  "mb-0.5 text-xs font-semibold text-ink-2" + (g.mono ? " font-mono" : "")
                }
              >
                {g.label}
              </div>
              <div className="divide-y divide-line-soft">
                {g.positions.map((p) => (
                  <PositionRow key={p.callsign} p={p} />
                ))}
              </div>
            </div>
          ))}
        </div>
      )}
      {board?.as_of && (
        <div className="mt-auto pt-2 font-mono text-[11px] text-ink-3">
          as of {hhmmZulu(board.as_of)}
        </div>
      )}
    </div>
  );
}
