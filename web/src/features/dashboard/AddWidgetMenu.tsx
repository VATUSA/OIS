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
import {Plus} from "lucide-react";

import {DATA_SOURCES} from "./sources";
import {STAT_METRICS} from "./stat-widgets";
import type {ViewId, Widget} from "./types";
import {VIEW_OPTIONS} from "./view-widgets";

const newId = () => crypto.randomUUID();

async function askIcao(prompt: ReturnType<typeof usePrompt>): Promise<string | null> {
  const raw = await prompt({ title: "Airport", label: "Airport (ICAO)", placeholder: "KJFK" });
  if (!raw) return null;
  const icao = raw.replace(/[^a-zA-Z0-9]/g, "").toUpperCase();
  return icao.length >= 3 ? icao : null;
}

export function AddWidgetMenu({ onAdd }: { onAdd: (widget: Widget) => void }) {
  const prompt = usePrompt();

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

  return (
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
        <DropdownMenuLabel>Map</DropdownMenuLabel>
        <DropdownMenuItem onSelect={() => onAdd({ id: newId(), kind: "map" })}>
          Flow map
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
