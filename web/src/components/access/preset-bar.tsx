import {ConfirmButton, cn} from "@ois/ui";
import {Wand2} from "lucide-react";

import {type AccessPreset, ACCESS_PRESETS} from "@/lib/presets";

/**
 * Shared presets bar for the access editors. Renders the preset bundles grouped into National and
 * Facility sections; the apply/toggle logic is injected by each editor (users apply roles + perms,
 * API keys apply perms only), so the presentation stays identical while the semantics differ.
 */
export function PresetBar({
  isApplied,
  onToggle,
  facility,
  facilities,
  onFacility,
  onRemoveAll,
  removeAllWarn,
  size = "sm",
}: {
  isApplied: (preset: AccessPreset) => boolean;
  onToggle: (preset: AccessPreset) => void;
  facility: string;
  facilities: { id: string; name: string }[];
  onFacility: (id: string) => void;
  onRemoveAll: () => void;
  removeAllWarn: string;
  size?: "sm" | "lg";
}) {
  const national = ACCESS_PRESETS.filter((p) => p.scope === "national");
  const facilityPresets = ACCESS_PRESETS.filter((p) => p.scope === "facility");
  const btn =
    size === "lg" ? "px-3 py-1.5 text-sm" : "px-2.5 py-1 text-xs";

  const Preset = ({ preset }: { preset: AccessPreset }) => {
    const needsFacility = preset.scope === "facility" && !facility;
    const applied = isApplied(preset);
    return (
      <button
        type="button"
        disabled={needsFacility}
        onClick={() => onToggle(preset)}
        title={needsFacility ? "Pick a facility first" : preset.description}
        className={cn(
          "rounded-md border font-medium transition-colors",
          btn,
          applied
            ? "border-primary/60 bg-primary/15 text-primary"
            : "bg-background hover:bg-accent",
          needsFacility && "cursor-not-allowed opacity-50",
        )}
      >
        {preset.label}
        {preset.scope === "facility" && facility ? ` · ${facility}` : ""}
      </button>
    );
  };

  return (
    <div className="flex flex-col gap-2 rounded-md border bg-muted/20 p-2">
      <div className="flex flex-wrap items-center gap-2">
        <span className="flex items-center gap-1 text-xs font-medium text-muted-foreground">
          <Wand2 className="size-3.5 text-primary" /> Presets
        </span>
        <span className="text-[10px] uppercase tracking-wide text-muted-foreground">National</span>
        {national.map((p) => (
          <Preset key={p.id} preset={p} />
        ))}
        <span className="mx-0.5 h-5 w-px bg-border" />
        <span className="text-[10px] uppercase tracking-wide text-muted-foreground">Facility</span>
        <select
          value={facility}
          onChange={(e) => onFacility(e.target.value)}
          title="Facility for a facility-scoped preset"
          className="h-7 rounded-md border border-input bg-background px-2 text-xs"
        >
          <option value="">Facility…</option>
          {facilities.map((f) => (
            <option key={f.id} value={f.id}>
              {f.id}
            </option>
          ))}
        </select>
        {facilityPresets.map((p) => (
          <Preset key={p.id} preset={p} />
        ))}
        <span className="mx-0.5 h-5 w-px bg-border" />
        <ConfirmButton size="sm" variant="ghost" warn={removeAllWarn} onConfirm={onRemoveAll}>
          Remove all
        </ConfirmButton>
      </div>
    </div>
  );
}
