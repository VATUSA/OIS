import {useState} from "react";
import {ArrowLeft, Lock, MapPinned, RefreshCw} from "lucide-react";
import {Button, Card, EmptyState, FilterBar, Input, QueryState, StatusPill, useConfirm} from "@ois/ui";

import {usePageHeader} from "@/components/shell/page-meta";
import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {useAirportSurface, useRepullFaaSurface} from "@/lib/airport-surface";
import {useRunway} from "@/lib/runway";
import {SurfaceMap} from "@/components/map/surface/SurfaceMap";

const normIcao = (s: string) => s.replace(/[^a-zA-Z0-9]/g, "").toUpperCase().slice(0, 4);

const SUBTITLE =
  "Draw and label gates/parking positions, ramp/apron areas, taxiways, and runway pavement for an airport. Runway ends — headings, lengths and balancer configuration — live on the runway balancer, not here.";

function RunwayList({ icao }: { icao: string }) {
  const runway = useRunway(icao);
  if (runway.isError || !runway.data?.ends.length) return null;
  return (
    <Card className="flex flex-col gap-2 p-4">
      <h2 className="text-xl font-bold">Runways</h2>
      <p className="text-sm text-ink-2">
        Read-only. Sourced from the bundled runway dataset — edit these on the runway balancer, not here.
      </p>
      <div className="flex flex-wrap gap-2 pt-1">
        {runway.data.ends.map((e) => (
          <StatusPill key={e.id} tone="neutral" className="font-mono">
            {e.id} · {String(e.hdg).padStart(3, "0")}° · {e.len} ft
          </StatusPill>
        ))}
      </div>
    </Card>
  );
}

function RepullFaaButton({ icao }: { icao: string }) {
  const repull = useRepullFaaSurface(icao);
  const confirm = useConfirm();
  const onClick = async () => {
    const ok = await confirm({
      title: `Re-pull ${icao} from FAA?`,
      description:
        "Replaces every FAA-sourced taxiway, ramp and runway at this airport with the bundled FAA data. Edits made to those rows are lost, and any you deleted come back. Hand-drawn (manual) geometry isn’t touched.",
      confirmText: "Re-pull",
    });
    if (ok) repull.mutate();
  };
  return (
    <Button variant="outline" size="sm" disabled={repull.isPending} onClick={onClick}>
      <RefreshCw className={`size-4 ${repull.isPending ? "animate-spin" : ""}`} />
      Re-pull from FAA
    </Button>
  );
}

function AirportSurfaceEditor({ icao, onBack }: { icao: string; onBack: () => void }) {
  const { data: me } = useMe();
  const surface = useAirportSurface(icao);

  const rows = surface.data
    ? [...surface.data.gates, ...surface.data.ramp_areas, ...surface.data.taxiways, ...surface.data.runways]
    : [];
  const editable = rows[0]?.editable ?? hasPermission(me, "flow.surface_data.update");

  return (
    <div className="flex flex-col gap-4">
      <FilterBar>
        <Button variant="ghost" size="sm" onClick={onBack}>
          <ArrowLeft className="size-4" />
          Change airport
        </Button>
        {surface.data && editable && (
          <div className="ml-auto">
            <RepullFaaButton icao={icao} />
          </div>
        )}
      </FilterBar>
      <QueryState
        isLoading={surface.isLoading}
        isError={surface.isError || (!surface.isLoading && !surface.data)}
        onRetry={() => surface.refetch()}
        error={`Couldn't load surface data for ${icao}.`}
        className="rounded-md border border-line py-16"
      >
        {surface.data && (
          <>
            <SurfaceMap icao={icao} surface={surface.data} editable={editable} />
            <RunwayList icao={icao} />
          </>
        )}
      </QueryState>
    </div>
  );
}

export function AirportSurfacePage() {
  const { data: me } = useMe();
  const canRead = hasPermission(me, "events.plan.read");
  const [icao, setIcao] = useState("");
  const [entry, setEntry] = useState("");

  usePageHeader({ title: icao ? `${icao} surface` : undefined, subtitle: SUBTITLE });

  if (!canRead) {
    return <EmptyState icon={Lock}>You don&apos;t have event planning access yet.</EmptyState>;
  }

  if (icao) {
    return <AirportSurfaceEditor key={icao} icao={icao} onBack={() => setIcao("")} />;
  }

  return (
    <div className="flex flex-col gap-4">
      <FilterBar>
        <Input
          aria-label="Airport ICAO"
          value={entry}
          onChange={(e) => setEntry(normIcao(e.target.value))}
          onKeyDown={(e) => e.key === "Enter" && entry.length === 4 && setIcao(entry)}
          placeholder="KDCA"
          className="h-8 w-32 font-mono uppercase"
        />
        <Button size="sm" disabled={entry.length !== 4} onClick={() => setIcao(entry)}>
          <MapPinned className="size-4" />
          Open
        </Button>
      </FilterBar>
      <EmptyState icon={MapPinned} className="rounded-md border border-line">
        Enter an airport ICAO to open its surface editor.
      </EmptyState>
    </div>
  );
}
