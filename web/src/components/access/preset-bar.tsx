import {ConfirmButton, cn, Select} from "@ois/ui";
import {Wand2} from "lucide-react";

import {type AccessPreset, ACCESS_PRESETS} from "@/lib/presets";

/**
 * Shared presets bar for the access editors. Renders the preset bundles grouped into National and
 * Facility sections; the apply/toggle logic is injected by each editor (users apply roles + perms,
 * API keys apply perms only), so the presentation stays identical while the semantics differ.
 */
export function PresetBar({
  isApplied,
  canApply = () => true,
  onToggle,
  facility,
  facilities,
  onFacility,
  onRemoveAll,
  removeAllWarn,
  size = "sm",
}: {
  isApplied: (preset: AccessPreset) => boolean;
  /** Whether the preset would grant anything; one that wouldn't renders disabled. */
  canApply?: (preset: AccessPreset) => boolean;
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
  const btn = size === "lg" ? "h-8 px-3 text-sm" : "h-7 px-2.5 text-xs";

  const Preset = ({ preset }: { preset: AccessPreset }) => {
    const needsFacility = preset.scope === "facility" && !facility;
    const nothingToGrant = !needsFacility && !canApply(preset);
    const disabled = needsFacility || nothingToGrant;
    const applied = isApplied(preset);
    return (
      <button
        type="button"
        disabled={disabled}
        onClick={() => onToggle(preset)}
        title={
          needsFacility
            ? "Pick a facility first"
            : nothingToGrant
              ? "Nothing in this preset that you can delegate"
              : preset.description
        }
        className={cn(
          "rounded-full border font-semibold transition-colors",
          btn,
          applied
            ? "border-brand/40 bg-brand-soft text-brand-ink"
            : "border-line bg-panel-2 text-ink-2 hover:bg-chip hover:text-ink",
          disabled && "cursor-not-allowed opacity-50",
        )}
      >
        {preset.label}
        {preset.scope === "facility" && facility ? ` · ${facility}` : ""}
      </button>
    );
  };

  return (
    <div className="flex flex-col gap-2 rounded-md border border-line bg-panel p-2.5">
      <div className="flex flex-wrap items-center gap-2">
        <span className="flex items-center gap-1 text-xs font-semibold text-ink-2">
          <Wand2 className="size-3.5 text-brand-ink" /> Presets
        </span>
        <span className="text-xs font-semibold text-ink-3">National</span>
        {national.map((p) => (
          <Preset key={p.id} preset={p} />
        ))}
        <span className="mx-0.5 h-5 w-px bg-line" />
        <span className="text-xs font-semibold text-ink-3">Facility</span>
        <Select
          size="sm"
          value={facility}
          onChange={(e) => onFacility(e.target.value)}
          title="Facility for a facility-scoped preset"
          aria-label="Facility for a facility-scoped preset"
          className="font-mono text-xs"
        >
          <option value="">Facility…</option>
          {facilities.map((f) => (
            <option key={f.id} value={f.id}>
              {f.id}
            </option>
          ))}
        </Select>
        {facilityPresets.map((p) => (
          <Preset key={p.id} preset={p} />
        ))}
        <ConfirmButton
          size="sm"
          variant="ghost"
          className="ml-auto"
          warn={removeAllWarn}
          onConfirm={onRemoveAll}
        >
          Remove all
        </ConfirmButton>
      </div>
    </div>
  );
}
