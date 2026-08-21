import {useEffect, useState} from "react";
import {Button, ConfirmButton, Input, Switch} from "@ois/ui";
import {ChevronDown, ChevronUp, Plus, Save, Trash2, X} from "lucide-react";

import {PALETTE} from "@/lib/facility-map/palette";
import {RULE_FIELDS, isNumericField, type ColorRule, type RuleCondition} from "@/lib/facility-map/rules";
import {
  useSaveFacilityMapConfig,
  type FacilityMapConfig,
  type UpsertFacilityMapConfig,
} from "@/lib/facility-map";

const newId = () => crypto.randomUUID();

const STRING_OPS = [
  { value: "in", label: "is" },
  { value: "prefix", label: "starts with" },
];
const NUM_OPS = [
  { value: "lt", label: "below" },
  { value: "gt", label: "above" },
  { value: "range", label: "between" },
];

const blankCondition = (): RuleCondition => ({ field: "arr", op: "in", values: [] });
const blankRule = (color: string): ColorRule => ({
  id: newId(),
  label: "",
  color,
  enabled: true,
  conditions: [blankCondition()],
});

function opsFor(field: string) {
  return isNumericField(field) ? NUM_OPS : STRING_OPS;
}
function valueHint(op: string): string {
  if (op === "range") return "min, max";
  if (op === "prefix") return "K";
  if (op === "lt" || op === "gt") return "10000";
  return "KIAD, KBWI";
}

/** A small palette swatch row; clicking one sets the color. */
function ColorSwatches({ value, onPick }: { value: string; onPick: (hex: string) => void }) {
  return (
    <div className="flex flex-wrap gap-1">
      {PALETTE.map((c) => (
        <button
          key={c.hex}
          type="button"
          title={c.label}
          onClick={() => onPick(c.hex)}
          className={`size-5 rounded-sm border transition-transform hover:scale-110 ${
            value.toLowerCase() === c.hex.toLowerCase() ? "ring-2 ring-foreground ring-offset-1 ring-offset-background" : ""
          }`}
          style={{ backgroundColor: c.hex }}
        />
      ))}
    </div>
  );
}

function ConditionRow({
  cond,
  onChange,
  onRemove,
}: {
  cond: RuleCondition;
  onChange: (c: RuleCondition) => void;
  onRemove: () => void;
}) {
  const ops = opsFor(cond.field);
  return (
    <div className="flex items-center gap-1.5">
      <select
        value={cond.field}
        onChange={(e) => {
          const field = e.target.value;
          // Reset op to a valid one for the new field kind, and clear values.
          const op = opsFor(field)[0].value;
          onChange({ field, op, values: [] });
        }}
        className="rounded border bg-background px-1.5 py-1 text-xs outline-none"
      >
        {RULE_FIELDS.map((f) => (
          <option key={f.value} value={f.value}>
            {f.label}
          </option>
        ))}
      </select>
      <select
        value={cond.op}
        onChange={(e) => onChange({ ...cond, op: e.target.value })}
        className="rounded border bg-background px-1.5 py-1 text-xs outline-none"
      >
        {ops.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
      <input
        // Uncontrolled + commit on blur so typing a comma list doesn't fight the array round-trip.
        key={`${cond.field}:${cond.op}`}
        defaultValue={cond.values.join(", ")}
        placeholder={valueHint(cond.op)}
        onBlur={(e) =>
          onChange({
            ...cond,
            values: e.target.value.split(",").map((s) => s.trim()).filter(Boolean),
          })
        }
        className="min-w-0 flex-1 rounded border bg-background px-1.5 py-1 text-xs outline-none"
      />
      <ConfirmButton
        size="icon"
        className="size-7"
        title="Remove condition"
        aria-label="Remove condition"
        onConfirm={onRemove}
        warn="Remove this condition?"
      >
        <X className="size-3.5" />
      </ConfirmButton>
    </div>
  );
}

function RuleCard({
  rule,
  index,
  count,
  onChange,
  onRemove,
  onMove,
}: {
  rule: ColorRule;
  index: number;
  count: number;
  onChange: (r: ColorRule) => void;
  onRemove: () => void;
  onMove: (dir: -1 | 1) => void;
}) {
  const setCond = (i: number, c: RuleCondition) =>
    onChange({ ...rule, conditions: rule.conditions.map((x, j) => (j === i ? c : x)) });
  return (
    <div className="rounded-lg border bg-background/60 p-2.5">
      <div className="mb-2 flex items-center gap-2">
        <Switch checked={rule.enabled} onCheckedChange={(v) => onChange({ ...rule, enabled: v })} />
        <span className="size-4 shrink-0 rounded-sm border" style={{ backgroundColor: rule.color }} />
        <Input
          value={rule.label}
          onChange={(e) => onChange({ ...rule, label: e.target.value })}
          placeholder="Rule name"
          className="h-7 flex-1 text-xs"
        />
        <button type="button" disabled={index === 0} onClick={() => onMove(-1)} title="Move up" className="text-muted-foreground disabled:opacity-30 hover:text-foreground">
          <ChevronUp className="size-4" />
        </button>
        <button type="button" disabled={index === count - 1} onClick={() => onMove(1)} title="Move down" className="text-muted-foreground disabled:opacity-30 hover:text-foreground">
          <ChevronDown className="size-4" />
        </button>
        <ConfirmButton
          size="icon"
          className="size-7"
          title="Delete rule"
          aria-label="Delete rule"
          onConfirm={onRemove}
          warn={rule.label ? `Delete the “${rule.label}” rule?` : "Delete this rule?"}
        >
          <Trash2 className="size-3.5" />
        </ConfirmButton>
      </div>
      <div className="mb-2">
        <ColorSwatches value={rule.color} onPick={(hex) => onChange({ ...rule, color: hex })} />
      </div>
      <div className="flex flex-col gap-1.5">
        {rule.conditions.map((c, i) => (
          <ConditionRow
            key={i}
            cond={c}
            onChange={(next) => setCond(i, next)}
            onRemove={() => onChange({ ...rule, conditions: rule.conditions.filter((_, j) => j !== i) })}
          />
        ))}
        <button
          type="button"
          onClick={() => onChange({ ...rule, conditions: [...rule.conditions, blankCondition()] })}
          className="self-start text-xs text-muted-foreground hover:text-foreground"
        >
          + condition
        </button>
      </div>
    </div>
  );
}

/**
 * The facility-map color-rule editor panel. Manages a local draft, streams it to `onPreview` for live
 * recoloring, and PUTs on Save. Rendered only when the caller is allowed to edit (server `editable`).
 */
export function RuleEditor({
  facilityId,
  initial,
  onPreview,
  onClose,
}: {
  facilityId: string;
  initial: FacilityMapConfig;
  onPreview: (draft: UpsertFacilityMapConfig) => void;
  onClose: () => void;
}) {
  const [rules, setRules] = useState<ColorRule[]>(initial.rules);
  const [defaultColor, setDefaultColor] = useState(initial.default_color);
  const save = useSaveFacilityMapConfig(facilityId);

  // Stream the draft up for live preview.
  useEffect(() => {
    onPreview({ rules, default_color: defaultColor });
  }, [rules, defaultColor, onPreview]);

  const dirty =
    JSON.stringify({ rules, default_color: defaultColor }) !==
    JSON.stringify({ rules: initial.rules, default_color: initial.default_color });

  const move = (i: number, dir: -1 | 1) => {
    const j = i + dir;
    if (j < 0 || j >= rules.length) return;
    const next = rules.slice();
    [next[i], next[j]] = [next[j], next[i]];
    setRules(next);
  };

  return (
    <div className="absolute right-0 top-0 z-[600] flex h-full w-[22rem] max-w-[calc(100vw-1rem)] flex-col border-l bg-background/95 shadow-xl backdrop-blur">
      <div className="flex items-center justify-between border-b px-3 py-2">
        <div className="text-sm font-semibold">{facilityId} color rules</div>
        <button type="button" onClick={onClose} title="Close" className="text-muted-foreground hover:text-foreground">
          <X className="size-4" />
        </button>
      </div>

      <div className="flex-1 overflow-y-auto p-3">
        <div className="mb-3 flex items-center justify-between gap-2 rounded-lg border bg-background/60 p-2.5">
          <div className="text-xs font-medium">Default (unmatched)</div>
          <ColorSwatches value={defaultColor} onPick={setDefaultColor} />
        </div>

        <div className="flex flex-col gap-2">
          {rules.map((r, i) => (
            <RuleCard
              key={r.id}
              rule={r}
              index={i}
              count={rules.length}
              onChange={(next) => setRules(rules.map((x, j) => (j === i ? next : x)))}
              onRemove={() => setRules(rules.filter((_, j) => j !== i))}
              onMove={(dir) => move(i, dir)}
            />
          ))}
        </div>

        <Button
          variant="outline"
          size="sm"
          className="mt-3 w-full"
          onClick={() => setRules([...rules, blankRule(PALETTE[rules.length % PALETTE.length].hex)])}
        >
          <Plus className="mr-1 size-4" /> Add rule
        </Button>
      </div>

      <div className="flex items-center gap-2 border-t px-3 py-2">
        <Button
          size="sm"
          className="flex-1"
          disabled={!dirty || save.isPending}
          onClick={() => save.mutate({ rules, default_color: defaultColor }, { onSuccess: onClose })}
        >
          <Save className="mr-1 size-4" /> {save.isPending ? "Saving…" : "Save"}
        </Button>
        {dirty ? (
          <ConfirmButton
            variant="ghost"
            size="sm"
            onConfirm={() => {
              setRules(initial.rules);
              setDefaultColor(initial.default_color);
            }}
          >
            Reset
          </ConfirmButton>
        ) : null}
      </div>
    </div>
  );
}
