import {useState} from "react";
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  usePrompt,
} from "@ois/ui";
import {Plus, X} from "lucide-react";

import {FacilityCombobox, type FacilityPick} from "@/components/facility-combobox";
import {defaultChartConfig} from "./chart-widget";
import {AIRPORT_KEY, DATA_SOURCES, DATA_SOURCES_BY_ID} from "./sources";
import {STAT_METRICS} from "./stat-widgets";
import type {ViewId, Widget} from "./types";
import {VIEW_OPTIONS} from "./view-widgets";

const newId = () => crypto.randomUUID();

/** The airport-scoped sources — the ones that also support a whole-facility scope. */
const AIRPORT_SOURCES = DATA_SOURCES.filter((s) => s.needsIcao);

async function askIcao(prompt: ReturnType<typeof usePrompt>): Promise<string | null> {
  const raw = await prompt({ title: "Airport", label: "Airport (ICAO)", placeholder: "KJFK" });
  if (!raw) return null;
  const icao = raw.replace(/[^a-zA-Z0-9]/g, "").toUpperCase();
  return icao.length >= 3 ? icao : null;
}

/** Pending facility action — set when a facility menu item is picked, cleared on facility select/cancel. */
type FacAction =
  | { type: "table" | "chart"; source: string }
  | { type: "atc" }
  | { type: "facility_map" };

export function AddWidgetMenu({ onAdd }: { onAdd: (widget: Widget) => void }) {
  const prompt = usePrompt();
  const [facAction, setFacAction] = useState<FacAction | null>(null);

  async function addView(view: ViewId) {
    const icao = await askIcao(prompt);
    if (icao) onAdd({ id: newId(), kind: "view", view, icao });
  }

  async function addTable(source: string, needsIcao: boolean) {
    if (needsIcao) {
      const icao = await askIcao(prompt);
      if (!icao) return;
      onAdd({ id: newId(), kind: "table", source, params: { icao } });
    } else {
      onAdd({ id: newId(), kind: "table", source });
    }
  }

  async function addAadc() {
    const icao = await askIcao(prompt);
    if (!icao) return;
    onAdd({ id: newId(), kind: "aadc", icao, bucketMin: 15, dimension: "status" });
  }

  async function addChart(sourceId: string, needsIcao: boolean) {
    const source = DATA_SOURCES_BY_ID[sourceId];
    if (!source) return;
    const cfg = defaultChartConfig(source);
    let params: { icaos?: string[] } | undefined;
    if (needsIcao) {
      const icao = await askIcao(prompt);
      if (!icao) return;
      params = { icaos: [icao] };
    }
    onAdd({ id: newId(), kind: "chart", source: sourceId, params, ...cfg });
  }

  /** Complete a pending facility action once a facility is chosen. */
  function onFacilitySelect(pick: FacilityPick) {
    const a = facAction;
    setFacAction(null);
    if (!a) return;
    if (a.type === "atc") {
      onAdd({ id: newId(), kind: "atc", facility: pick });
      return;
    }
    if (a.type === "facility_map") {
      // Color-coded aircraft + ATC + routes, on by default (matches the standalone map).
      onAdd({ id: newId(), kind: "facility_map", facilityId: pick.id, atc: true, routes: true });
      return;
    }
    const source = DATA_SOURCES_BY_ID[a.source];
    if (!source) return;
    if (a.type === "table") {
      onAdd({ id: newId(), kind: "table", source: a.source, params: { facility: pick } });
    } else {
      // Compare the facility's airports side by side by default.
      onAdd({
        id: newId(),
        kind: "chart",
        source: a.source,
        params: { facility: pick },
        ...defaultChartConfig(source),
        x: AIRPORT_KEY,
        y: [],
        aggregate: "count",
      });
    }
  }

  return (
    <>
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button size="sm">
          <Plus />
          Add widget
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="max-h-[70vh] w-56 overflow-y-auto">
        <DropdownMenuLabel>Stat tiles</DropdownMenuLabel>
        {STAT_METRICS.map((m) => (
          <DropdownMenuItem
            key={m.id}
            onSelect={() => onAdd({ id: newId(), kind: "stat", metric: m.id })}
          >
            {m.label}
          </DropdownMenuItem>
        ))}
        <DropdownMenuSeparator />
        <DropdownMenuLabel>Airport views</DropdownMenuLabel>
        {VIEW_OPTIONS.map((v) => (
          <DropdownMenuItem key={v.id} onSelect={() => void addView(v.id)}>
            {v.label}
          </DropdownMenuItem>
        ))}
        <DropdownMenuSeparator />
        <DropdownMenuLabel>Tables</DropdownMenuLabel>
        {DATA_SOURCES.map((s) => (
          <DropdownMenuItem key={s.id} onSelect={() => void addTable(s.id, s.needsIcao)}>
            {s.label}
            {s.needsIcao && <span className="ml-auto text-xs text-muted-foreground">airport</span>}
          </DropdownMenuItem>
        ))}
        <DropdownMenuSeparator />
        <DropdownMenuLabel>Charts</DropdownMenuLabel>
        {DATA_SOURCES.filter((s) => s.fields.some((f) => f.type === "number")).map((s) => (
          <DropdownMenuItem key={s.id} onSelect={() => void addChart(s.id, s.needsIcao)}>
            {s.label}
            {s.needsIcao && <span className="ml-auto text-xs text-muted-foreground">airport</span>}
          </DropdownMenuItem>
        ))}
        <DropdownMenuItem onSelect={() => void addAadc()}>
          Arrival demand chart (AADC)
          <span className="ml-auto text-xs text-muted-foreground">airport</span>
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuLabel>Facilities (ARTCC / TRACON)</DropdownMenuLabel>
        {AIRPORT_SOURCES.map((s) => (
          <DropdownMenuItem key={`fac-t-${s.id}`} onSelect={() => setFacAction({ type: "table", source: s.id })}>
            {s.label}
            <span className="ml-auto text-xs text-muted-foreground">table</span>
          </DropdownMenuItem>
        ))}
        {AIRPORT_SOURCES.filter((s) => s.fields.some((f) => f.type === "number")).map((s) => (
          <DropdownMenuItem key={`fac-c-${s.id}`} onSelect={() => setFacAction({ type: "chart", source: s.id })}>
            {s.label} — compare
            <span className="ml-auto text-xs text-muted-foreground">chart</span>
          </DropdownMenuItem>
        ))}
        <DropdownMenuItem onSelect={() => setFacAction({ type: "atc" })}>
          Online ATC positions
          <span className="ml-auto text-xs text-muted-foreground">atc</span>
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuLabel>Map</DropdownMenuLabel>
        <DropdownMenuItem onSelect={() => onAdd({ id: newId(), kind: "map" })}>
          Flow map
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => setFacAction({ type: "facility_map" })}>
          Facility map
          <span className="ml-auto text-xs text-muted-foreground">ARTCC</span>
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuLabel>Layout</DropdownMenuLabel>
        <DropdownMenuItem
          onSelect={() =>
            onAdd({ id: newId(), kind: "text", content: "New heading", size: "lg", align: "left" })
          }
        >
          Text / heading
        </DropdownMenuItem>
        <DropdownMenuItem
          onSelect={() => onAdd({ id: newId(), kind: "divider", orientation: "horizontal" })}
        >
          Divider
          <span className="ml-auto text-xs text-muted-foreground">horizontal</span>
        </DropdownMenuItem>
        <DropdownMenuItem
          onSelect={() => onAdd({ id: newId(), kind: "divider", orientation: "vertical" })}
        >
          Divider
          <span className="ml-auto text-xs text-muted-foreground">vertical</span>
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>

    {facAction && (
      <div
        className="fixed inset-0 z-[900] flex items-start justify-center bg-black/40 pt-32"
        onClick={() => setFacAction(null)}
      >
        <div
          className="w-80 rounded-lg border bg-background p-4 shadow-2xl"
          onClick={(e) => e.stopPropagation()}
        >
          <div className="mb-3 flex items-center justify-between">
            <span className="text-sm font-semibold">Add for a facility</span>
            <button
              type="button"
              aria-label="Cancel"
              onClick={() => setFacAction(null)}
              className="text-muted-foreground hover:text-foreground"
            >
              <X className="size-4" />
            </button>
          </div>
          <FacilityCombobox autoFocus onSelect={onFacilitySelect} />
          <p className="mt-2 text-xs text-muted-foreground">
            Covers every airport in the ARTCC or TRACON.
          </p>
        </div>
      </div>
    )}
    </>
  );
}
