import {Button} from "@ois/ui";
import {X} from "lucide-react";

import type {AirportGate} from "@/lib/airport-surface";

/**
 * What the X-Plane extract knows about a stand, shown when one is clicked (VATUSA/OIS#541).
 *
 * This exists because clicking a stand used to do **nothing at all** for a user without
 * `flow.surface_data.update`: the click handler returned early, so their only route to a stand's name
 * was the hover tooltip. That left read-only users — most users — unable to identify a stand.
 *
 * Deliberately read-only. Every field below describes what the source said, not what an operator
 * intends, so there is nothing here to edit; users who *can* edit get the editor panel instead.
 */
export function StandDetailCard({gate, onClose}: {gate: AirportGate; onClose: () => void}) {
  // Only what this stand actually has. A row per absent field would make a hand-entered stand — which
  // has none of the X-Plane detail — render as a column of dashes.
  const rows: [string, string][] = [];
  if (gate.kind) rows.push(["Type", gate.kind.replace(/_/g, " ")]);
  if (gate.size_code) rows.push(["Size", `ICAO ${gate.size_code}`]);
  if (gate.heading != null) rows.push(["Heading", `${Math.round(gate.heading)}°`]);
  if (gate.operation_type) rows.push(["Operation", gate.operation_type.replace(/_/g, " ")]);
  if (gate.aircraft_classes?.length) rows.push(["Aircraft", gate.aircraft_classes.join(", ")]);
  if (gate.airline_codes?.length) {
    rows.push(["Airlines", gate.airline_codes.map((c) => c.toUpperCase()).join(", ")]);
  }
  rows.push(["Source", gate.source]);

  return (
    <div className="flex flex-col gap-3 rounded-md border border-line bg-panel p-3">
      <div className="flex items-start justify-between gap-2">
        <span className="font-mono text-sm font-bold text-ink">{gate.name}</span>
        <Button variant="ghost" size="sm" aria-label="Close stand details" onClick={onClose}>
          <X className="size-4" />
        </Button>
      </div>
      <dl className="flex flex-col gap-1 text-xs">
        {rows.map(([label, value]) => (
          <div key={label} className="flex items-baseline justify-between gap-3">
            <dt className="text-ink-3">{label}</dt>
            <dd className="text-right font-mono text-ink-2">{value}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}
