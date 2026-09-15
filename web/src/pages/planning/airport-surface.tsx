import {useState} from "react";
import {ArrowLeft, MapPinned, RefreshCw} from "lucide-react";
import {Button, Card, CardContent, Input} from "@ois/ui";

import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {useAirportSurface, useRepullFaaSurface} from "@/lib/airport-surface";
import {useRunway} from "@/lib/runway";
import {SurfaceMap} from "@/components/map/surface/SurfaceMap";

const normIcao = (s: string) => s.replace(/[^a-zA-Z0-9]/g, "").toUpperCase().slice(0, 4);

function RunwayList({ icao }: { icao: string }) {
  const runway = useRunway(icao);
  if (runway.isError || !runway.data?.ends.length) return null;
  return (
    <Card>
      <CardContent className="flex flex-col gap-2 pt-6">
        <span className="text-sm font-semibold">Runways (read-only)</span>
        <p className="text-xs text-muted-foreground">
          Sourced from the bundled runway dataset — edit these on the runway balancer, not here.
        </p>
        <div className="flex flex-wrap gap-2 pt-1">
          {runway.data.ends.map((e) => (
            <span key={e.id} className="rounded-md border bg-muted/30 px-2 py-1 text-xs">
              {e.id} · {String(e.hdg).padStart(3, "0")}° · {e.len} ft
            </span>
          ))}
        </div>
      </CardContent>
    </Card>
  );
}

function RepullFaaButton({ icao }: { icao: string }) {
  const repull = useRepullFaaSurface(icao);
  return (
    <Button variant="outline" size="sm" disabled={repull.isPending} onClick={() => repull.mutate()}>
      <RefreshCw className={`size-4 ${repull.isPending ? "animate-spin" : ""}`} />
      Re-pull from FAA
    </Button>
  );
}

function AirportSurfaceEditor({ icao }: { icao: string }) {
  const { data: me } = useMe();
  const surface = useAirportSurface(icao);

  if (surface.isLoading) {
    return (
      <Card>
        <CardContent className="py-16 text-center text-sm text-muted-foreground">Loading…</CardContent>
      </Card>
    );
  }
  if (surface.isError || !surface.data) {
    return (
      <Card>
        <CardContent className="py-16 text-center text-sm text-muted-foreground">
          Couldn&apos;t load surface data for {icao}.
        </CardContent>
      </Card>
    );
  }

  const rows = [...surface.data.gates, ...surface.data.ramp_areas, ...surface.data.taxiways];
  const editable = rows[0]?.editable ?? hasPermission(me, "flow.surface_data.update");

  return (
    <div className="flex flex-col gap-4">
      {editable && (
        <div className="flex justify-end">
          <RepullFaaButton icao={icao} />
        </div>
      )}
      <SurfaceMap icao={icao} surface={surface.data} editable={editable} />
      <RunwayList icao={icao} />
    </div>
  );
}

export function AirportSurfacePage() {
  const { data: me } = useMe();
  const canRead = hasPermission(me, "events.plan.read");
  const [icao, setIcao] = useState("");
  const [entry, setEntry] = useState("");

  if (!canRead) {
    return (
      <Card>
        <CardContent className="py-16 text-center text-sm text-muted-foreground">
          You don&apos;t have event planning access yet.
        </CardContent>
      </Card>
    );
  }

  if (icao) {
    return (
      <div className="mx-auto flex w-full max-w-5xl flex-col gap-6">
        <Button variant="ghost" className="w-fit px-2" onClick={() => setIcao("")}>
          <ArrowLeft className="size-4" />
          Change airport
        </Button>
        <AirportSurfaceEditor key={icao} icao={icao} />
      </div>
    );
  }

  return (
    <div className="mx-auto flex w-full max-w-5xl flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Airport surface data</h1>
        <p className="text-muted-foreground">
          Draw and label gates/parking positions, ramp/apron areas, and taxiways for an airport.
          Runways are shown read-only here — edit those on the runway balancer.
        </p>
      </div>
      <div className="flex flex-wrap items-end gap-3">
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-muted-foreground">Airport ICAO</span>
          <Input
            value={entry}
            onChange={(e) => setEntry(normIcao(e.target.value))}
            onKeyDown={(e) => e.key === "Enter" && entry.length === 4 && setIcao(entry)}
            placeholder="KDCA"
            className="w-32"
          />
        </label>
        <Button disabled={entry.length !== 4} onClick={() => setIcao(entry)}>
          <MapPinned className="size-4" />
          Open
        </Button>
      </div>
    </div>
  );
}
