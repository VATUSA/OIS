import {useAtc, type AtcPosition} from "@/lib/fca";
import {facilityAirports, facilityKindLabel, useFacilityDirectory} from "@/lib/facilities";
import {onlineFor, ratingLabel} from "@/lib/atc-format";
import {hhmmZulu} from "@/lib/time";

import type {AtcWidget} from "./types";
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

/** Online ATC for a facility: its center/approach positions + its airports' ground stacks. */
export function AtcWidgetView({ widget }: { widget: AtcWidget }) {
  const { facility } = widget;
  const { data: board, isLoading, isFetching, dataUpdatedAt, refetch } = useAtc(true);
  const dir = useFacilityDirectory();
  useReportWidgetStatus(isFetching, dataUpdatedAt, refetch);

  const facilityPositions: AtcPosition[] =
    facility.kind === "artcc"
      ? (board?.centers.find((c) => c.id === facility.id)?.positions ?? [])
      : (board?.tracons.find((t) => t.id === facility.id)?.positions ?? []);

  const memberIcaos = facilityAirports(dir.data, facility.id);
  const airportGroups = (board?.airports ?? [])
    .filter((a) => memberIcaos.includes(a.icao))
    .sort((a, b) => a.icao.localeCompare(b.icao));

  const anyOnline = facilityPositions.length > 0 || airportGroups.length > 0;
  const facilityLabel = facility.kind === "artcc" ? "Center" : "Approach / Departure";

  return (
    <div className="flex h-full flex-col overflow-y-auto p-3 text-ink">
      {isLoading && !board ? (
        <p className="text-sm text-ink-3">Loading…</p>
      ) : !anyOnline ? (
        <p className="text-sm text-ink-3">
          No online ATC for {facility.id} ({facilityKindLabel(facility.kind)}).
        </p>
      ) : (
        <div className="flex flex-col gap-3">
          {facilityPositions.length > 0 && (
            <div>
              <div className="mb-0.5 text-xs font-semibold text-ink-2">
                {facilityLabel}
              </div>
              <div className="divide-y divide-line-soft">
                {facilityPositions.map((p) => (
                  <PositionRow key={p.callsign} p={p} />
                ))}
              </div>
            </div>
          )}
          {airportGroups.map((ap) => (
            <div key={ap.icao}>
              <div className="mb-0.5 font-mono text-xs font-semibold text-ink-2">{ap.icao}</div>
              <div className="divide-y divide-line-soft">
                {ap.positions.map((p) => (
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
