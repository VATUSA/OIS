import type {ComponentType} from "react";
import {cn, toneText, type Tone} from "@ois/ui";

import {useFcas} from "@/lib/fca";
import {useFeedStatus} from "@/lib/feed";
import {useGdps} from "@/lib/gdp";
import {useHistoricalAt} from "@/lib/historical-context";
import {useHistPilotCount} from "@/lib/historical";
import {useGroundStops, usePrograms, useTmis} from "@/lib/tmu";

import type {StatMetricId} from "./types";

/** A stat tile — the MetricCard type ramp (label · big 700 mono number · sub) inside the widget frame. */
function Tile({
  label,
  value,
  sub,
  tone,
}: {
  label: string;
  value: number | string;
  sub?: string;
  tone?: Tone;
}) {
  return (
    <div className="flex h-full flex-col justify-center gap-1.5 px-1">
      <span className="text-xs text-ink-2">{label}</span>
      <span className={cn("font-mono text-4xl font-bold leading-none tracking-tight", tone ? toneText[tone] : "text-ink")}>
        {value}
      </span>
      {sub && <span className="text-xs text-ink-3">{sub}</span>}
    </div>
  );
}

// One component per metric — each calls exactly one hook, so a StatWidget that swaps metric
// unmounts one and mounts another (React-hooks-safe) rather than reordering hook calls.

/** Config metrics (programs/TMIs/ground stops/GDPs/FCAs) have no point-in-time historical form —
 * config history is out of scope — so they read "n/a" in replay rather than showing current config. */
function NaTile({ label }: { label: string }) {
  return <Tile label={label} value="—" sub="n/a in replay" tone="neutral" />;
}

function PilotsStat() {
  const at = useHistoricalAt();
  const live = useFeedStatus();
  const hist = useHistPilotCount(at);
  if (at != null) {
    return (
      <Tile label="Pilots online" value={hist.data ?? "—"} sub="in replay" tone="brand" />
    );
  }
  const data = live.data;
  return (
    <Tile
      label="Pilots online"
      value={data?.pilots ?? "—"}
      sub={data ? (data.healthy ? "feed live" : "feed down") : "connecting…"}
      tone={data && !data.healthy ? "neutral" : "good"}
    />
  );
}

function ProgramsStat() {
  const at = useHistoricalAt();
  const { data } = usePrograms();
  if (at != null) return <NaTile label="Metering programs" />;
  return <Tile label="Metering programs" value={data?.length ?? "—"} />;
}

// TMIs / ground stops / GDPs / FCAs are historized (soft-delete + published-window), so their hooks
// are mode-aware and these tiles show the count that was active at the scrubber instant.

function TmisStat() {
  const { data } = useTmis();
  const n = data?.length ?? 0;
  return <Tile label="Active TMIs" value={data ? n : "—"} tone={n > 0 ? "warn" : undefined} />;
}

function GroundStopsStat() {
  const { data } = useGroundStops();
  const n = data?.length ?? 0;
  return (
    <Tile label="Ground stops" value={data ? n : "—"} tone={n > 0 ? "bad" : undefined} />
  );
}

function GdpsStat() {
  const { data } = useGdps();
  const n = data?.length ?? 0;
  return (
    <Tile label="Ground delay programs" value={data ? n : "—"} tone={n > 0 ? "warn" : undefined} />
  );
}

function FcasStat() {
  const { data } = useFcas();
  return <Tile label="Flow constrained areas" value={data?.length ?? "—"} />;
}

/** The metrics a stat widget can show — also drives the add-widget menu. */
export const STAT_METRICS: {
  id: StatMetricId;
  label: string;
  Component: ComponentType;
}[] = [
  { id: "pilots", label: "Pilots online", Component: PilotsStat },
  { id: "programs", label: "Metering programs", Component: ProgramsStat },
  { id: "tmis", label: "Active TMIs", Component: TmisStat },
  { id: "ground-stops", label: "Ground stops", Component: GroundStopsStat },
  { id: "gdps", label: "Ground delay programs", Component: GdpsStat },
  { id: "fcas", label: "Flow constrained areas", Component: FcasStat },
];

export function StatWidgetView({ metric }: { metric: StatMetricId }) {
  const entry = STAT_METRICS.find((m) => m.id === metric);
  if (!entry) return <Tile label="Unknown metric" value="—" />;
  const Component = entry.Component;
  return <Component />;
}
