import {useState} from "react";
import {Button, Card, CardContent, Input} from "@ois/ui";
import {Plus, X} from "lucide-react";

import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {formatZulu} from "@/lib/time";
import {
  type CreateGroundStop,
  type GroundStop,
  useCreateGroundStop,
  useDeleteGroundStop,
  useGroundStops,
} from "@/lib/tmu";

const COLS =
  "grid grid-cols-[minmax(0,1fr)_minmax(0,2fr)_minmax(0,1fr)_auto] items-end gap-3";

function Head({ children }: { children: React.ReactNode }) {
  return (
    <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
      {children}
    </span>
  );
}

const EMPTY: CreateGroundStop = { airport: "", scope: "", until: "" };

function CreateForm() {
  const create = useCreateGroundStop();
  const [form, setForm] = useState<CreateGroundStop>(EMPTY);
  const [error, setError] = useState<string | null>(null);

  function set<K extends keyof CreateGroundStop>(key: K, value: string) {
    setForm((f) => ({ ...f, [key]: value }));
  }

  function submit() {
    const airport = (form.airport ?? "").replace(/[^a-zA-Z0-9]/g, "");
    if (airport.length < 3 || airport.length > 4) {
      setError("Enter a 3–4 character airport ICAO.");
      return;
    }
    setError(null);
    create.mutate(
      { airport, scope: form.scope, until: form.until },
      {
        onSuccess: () => setForm(EMPTY),
        onError: () => setError("Couldn’t issue the ground stop — check the UNTIL time."),
      },
    );
  }

  return (
    <Card>
      <CardContent className="flex flex-col gap-3 pt-6">
        <p className="text-sm text-muted-foreground">
          Holds GROUND departures into the named airport that originate inside the
          scoped ARTCC/FIR(s). Leave SCOPE blank for a field-wide stop.
        </p>
        <div className="overflow-x-auto">
          <div className={`${COLS} min-w-[560px]`}>
            <Head>Airport</Head>
            <Head>Scope (ARTCC/FIR)</Head>
            <Head>Until (Zxxxx)</Head>
            <span />

            <Input
              className="font-mono uppercase"
              maxLength={4}
              placeholder="KDCA"
              value={form.airport}
              onChange={(e) => set("airport", e.target.value)}
            />
            <Input
              className="font-mono uppercase"
              placeholder="ZTL ZJX"
              value={form.scope ?? ""}
              onChange={(e) => set("scope", e.target.value)}
            />
            <Input
              className="font-mono"
              placeholder="0200z"
              value={form.until ?? ""}
              onChange={(e) => set("until", e.target.value)}
            />
            <button
              type="button"
              title="Clear row"
              onClick={() => {
                setForm(EMPTY);
                setError(null);
              }}
              className="flex size-9 items-center justify-center rounded-md text-muted-foreground transition-colors hover:text-destructive"
            >
              <X className="size-4" />
            </button>
          </div>
        </div>
        <div className="flex items-center justify-between gap-3">
          <p className="text-sm text-destructive">{error ?? ""}</p>
          <Button disabled={create.isPending} onClick={submit}>
            <Plus />
            Add ground stop
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

function GroundStopRow({ gs, canDelete }: { gs: GroundStop; canDelete: boolean }) {
  const del = useDeleteGroundStop();
  return (
    <tr className="border-t">
      <td className="py-2 pr-3 font-mono font-medium">{gs.airport}</td>
      <td className="py-2 pr-3 font-mono text-xs">
        {gs.scope ? gs.scope : <span className="text-muted-foreground">All departures</span>}
      </td>
      <td className="py-2 pr-3 font-mono text-xs">
        {gs.until ? `${gs.until}z` : <span className="text-muted-foreground">UFN</span>}
      </td>
      <td className="py-2 pr-3 font-mono text-xs text-muted-foreground">
        {formatZulu(gs.updated_at)}
        {gs.updated_by ? ` · ${gs.updated_by}` : ""}
      </td>
      <td className="py-2 text-right">
        {canDelete && (
          <button
            type="button"
            title="Cancel ground stop"
            disabled={del.isPending}
            onClick={() => del.mutate(gs.id)}
            className="text-muted-foreground transition-colors hover:text-destructive"
          >
            <X className="size-4" />
          </button>
        )}
      </td>
    </tr>
  );
}

export function GroundStopsTab() {
  const { data: me } = useMe();
  const stops = useGroundStops();
  const canCreate = hasPermission(me, "tmu.groundstop.create");
  const canDelete = hasPermission(me, "tmu.groundstop.delete");

  return (
    <div className="flex flex-col gap-6">
      {canCreate && <CreateForm />}

      <Card>
        <CardContent className="pt-6">
          {stops.isError ? (
            <p className="py-8 text-center text-sm text-muted-foreground">
              Couldn&apos;t load ground stops.
            </p>
          ) : !stops.data ? (
            <p className="py-8 text-center text-sm text-muted-foreground">Loading…</p>
          ) : stops.data.length === 0 ? (
            <p className="py-8 text-center text-sm text-muted-foreground">
              No active ground stops.
            </p>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="text-left text-xs uppercase tracking-wide text-muted-foreground">
                    <th className="pb-2 pr-3 font-medium">Airport</th>
                    <th className="pb-2 pr-3 font-medium">Scope (ARTCC/FIR)</th>
                    <th className="pb-2 pr-3 font-medium">Until (Zxxxx)</th>
                    <th className="pb-2 pr-3 font-medium">Updated</th>
                    <th className="pb-2" />
                  </tr>
                </thead>
                <tbody>
                  {stops.data.map((gs) => (
                    <GroundStopRow key={gs.id} gs={gs} canDelete={canDelete} />
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
