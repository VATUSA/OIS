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

import {STAT_METRICS} from "./stat-widgets";
import type {ViewId, Widget} from "./types";
import {VIEW_OPTIONS} from "./view-widgets";

const newId = () => crypto.randomUUID();

export function AddWidgetMenu({ onAdd }: { onAdd: (widget: Widget) => void }) {
  const prompt = usePrompt();

  async function addView(view: ViewId) {
    const raw = await prompt({
      title: "Add airport view",
      label: "Airport (ICAO)",
      placeholder: "KJFK",
    });
    if (!raw) return;
    const icao = raw.replace(/[^a-zA-Z0-9]/g, "").toUpperCase();
    if (icao.length < 3) return;
    onAdd({ id: newId(), kind: "view", view, icao });
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
        <DropdownMenuLabel>Map</DropdownMenuLabel>
        <DropdownMenuItem onSelect={() => onAdd({ id: newId(), kind: "map" })}>
          Flow map
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
