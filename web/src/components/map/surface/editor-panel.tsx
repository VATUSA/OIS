import {Button, Input} from "@ois/ui";

import type {LatLng} from "../lib/geo";
import {MIN_SURFACE_POINTS, isPolygonKind, type SurfaceKind} from "./layers";

const SELECT_CLASS =
  "h-9 rounded-md border border-input bg-background px-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring";

/** The shape being drawn or edited. `points` never carries a polygon's closing duplicate — that's
 * added only when building the save request. */
export interface SurfaceDraft {
  kind: SurfaceKind;
  id?: string;
  name: string;
  rampKind: "ramp" | "apron";
  points: LatLng[];
  /** A polygon's (ramp/apron area or taxiway) rings beyond the first (e.g. a hole) — this editor only draws/edits the
   * outer ring, so these are carried through unedited and must be resent as-is on save, not
   * silently dropped. Always empty for a new draft or a gate. */
  extraRings?: LatLng[][];
}

/** Example name for each kind, shown as the name field's placeholder. */
const NAME_HINT: Record<SurfaceKind, string> = {
  gate: "A1",
  taxiway: "Alpha",
  ramp: "North apron",
  runway: "01/19",
};

const HINT: Record<SurfaceKind, string> = {
  gate: "Click the map to place the gate.",
  taxiway: "Click to add corners of the taxiway pavement outline, then Close shape once you have at least 3.",
  ramp: "Click to add corners of the ramp/apron boundary, then Close shape once you have at least 3.",
  runway: "Click to add corners of the runway pavement outline, then Close shape once you have at least 3.",
};

/**
 * The side panel for the in-progress draft — mirrors `fca/editors.tsx`'s `DraftEditor`: a drawing
 * hint + Finish/Cancel while accumulating points, then a name (+ kind, for ramp areas) form with
 * Save/Delete/Cancel once the shape is finalized.
 */
export function SurfaceEditorPanel({
  draft,
  phase,
  pending,
  onNameChange,
  onRampKindChange,
  onFinishDraw,
  onSave,
  onCancel,
  onDelete,
}: {
  draft: SurfaceDraft;
  phase: "draw" | "edit";
  pending: boolean;
  onNameChange: (name: string) => void;
  onRampKindChange: (kind: "ramp" | "apron") => void;
  onFinishDraw: () => void;
  onSave: () => void;
  onCancel: () => void;
  onDelete?: () => void;
}) {
  if (phase === "draw") {
    const canFinish = draft.points.length >= MIN_SURFACE_POINTS[draft.kind];
    return (
      <div className="flex flex-col gap-3 rounded-md border bg-background/95 p-3 shadow-lg">
        <p className="text-sm text-muted-foreground">{HINT[draft.kind]}</p>
        <p className="text-xs text-muted-foreground">{draft.points.length} point(s) placed</p>
        <div className="flex items-center justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={onCancel}>
            Cancel
          </Button>
          {draft.kind !== "gate" && (
            <Button size="sm" disabled={!canFinish} onClick={onFinishDraw}>
              {isPolygonKind(draft.kind) ? "Close shape" : "Finish"}
            </Button>
          )}
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-3 rounded-md border bg-background/95 p-3 shadow-lg">
      <label className="flex flex-col gap-1 text-xs">
        <span className="text-muted-foreground">Name</span>
        <Input
          autoFocus
          value={draft.name}
          onChange={(e) => onNameChange(e.target.value)}
          placeholder={NAME_HINT[draft.kind]}
        />
      </label>
      {draft.kind === "ramp" && (
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-muted-foreground">Kind</span>
          <select
            className={SELECT_CLASS}
            value={draft.rampKind}
            onChange={(e) => onRampKindChange(e.target.value as "ramp" | "apron")}
          >
            <option value="ramp">Ramp</option>
            <option value="apron">Apron</option>
          </select>
        </label>
      )}
      <div className="flex items-center justify-between gap-2">
        {onDelete ? (
          <Button variant="ghost" size="sm" className="text-destructive" onClick={onDelete} disabled={pending}>
            Delete
          </Button>
        ) : (
          <span />
        )}
        <div className="flex items-center gap-2">
          <Button variant="ghost" size="sm" onClick={onCancel}>
            Cancel
          </Button>
          <Button size="sm" disabled={!draft.name.trim() || pending} onClick={onSave}>
            Save
          </Button>
        </div>
      </div>
    </div>
  );
}
