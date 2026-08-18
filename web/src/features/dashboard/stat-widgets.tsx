import type {ComponentType} from "react";

import {useFcas} from "@/lib/fca";
import {useFeedStatus} from "@/lib/feed";
import {useGdps} from "@/lib/gdp";
import {useHistoricalAt} from "@/lib/historical-context";
import {useHistPilotCount} from "@/lib/historical";
import {useGroundStops, usePrograms, useTmis} from "@/lib/tmu";

import type {StatMetricId} from "./types";

function Tile({
  label,
  value,
  sub,
  tone,
}: {
  label: string;
  value: number | string;
  sub?: string;
  tone?: string;
}) {
  return (
    <div className="flex h-full flex-col justify-center gap-1 px-1">
      <span className={"text-4xl font-semibold tabular-nums " + (tone ?? "")}>{value}</span>
      <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
        {label}
      </span>
      {sub && <span className="text-xs text-muted-foreground/80">{sub}</span>}
    </div>
  );
}

// One component per metric — each calls exactly one hook, so a StatWidget that swaps metric
// unmounts one and mounts another (React-hooks-safe) rather than reordering hook calls.

/** Config metrics (programs/TMIs/ground stops/GDPs/FCAs) have no point-in-time historical form —
 * config history is out of scope — so they read "n/a" in replay rather than showing current config. */
function NaTile({ label }: { label: string }) {
  return <Tile label={label} value="—" sub="n/a in replay" tone="text-muted-foreground" />;
}

function PilotsStat() {
  const at = useHistoricalAt();
  const live = useFeedStatus();
  const hist = useHistPilotCount(at);
  if (at != null) {
    return (
      <Tile label="Pilots online" value={hist.data ?? "—"} sub="in replay" tone="text-sky-400" />
    );
  }
  const data = live.data;
  return (
    <Tile
      label="Pilots online"
      value={data?.pilots ?? "—"}
      sub={data ? (data.healthy ? "feed live" : "feed down") : "connecting…"}
      tone={data && !data.healthy ? "text-muted-foreground" : "text-emerald-500"}
    />
  );
}

function ProgramsStat() {
  const at = useHistoricalAt();
  const { data } = usePrograms();
  if (at != null) return <NaTile label="Metering programs" />;
  return <Tile label="Metering programs" value={data?.length ?? "—"} />;
}

function TmisStat() {
  const at = useHistoricalAt();
  const { data } = useTmis();
  const n = data?.length ?? 0;
  if (at != null) return <NaTile label="Active TMIs" />;
  return <Tile label="Active TMIs" value={data ? n : "—"} tone={n > 0 ? "text-amber-500" : ""} />;
}

function GroundStopsStat() {
  const at = useHistoricalAt();
  const { data } = useGroundStops();
  const n = data?.length ?? 0;
  if (at != null) return <NaTile label="Ground stops" />;
  return (
    <Tile label="Ground stops" value={data ? n : "—"} tone={n > 0 ? "text-destructive" : ""} />
  );
}

function GdpsStat() {
  const at = useHistoricalAt();
  const { data } = useGdps();
  const n = data?.length ?? 0;
  if (at != null) return <NaTile label="Ground delay programs" />;
  return (
    <Tile label="Ground delay programs" value={data ? n : "—"} tone={n > 0 ? "text-amber-500" : ""} />
  );
}

function FcasStat() {
  const at = useHistoricalAt();
  const { data } = useFcas();
  if (at != null) return <NaTile label="Flow constrained areas" />;
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
