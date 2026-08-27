import {FacilityMapView} from "@/pages/facility-map";
import type {FacilityMapWidget} from "./types";

/** Dashboard widget: a per-facility TMU map with color-coded aircraft. Fills the cell and shows the
 * full toolbar (Recenter / layer toggles / edit routes / edit rules / embed) by default; the widget's
 * edit config can hide the toolbar for a clean, minimal map. */
export function FacilityMapWidgetView({
  widget,
  editing,
  onChange,
}: {
  widget: FacilityMapWidget;
  editing: boolean;
  onChange: (id: string, patch: Record<string, unknown>) => void;
}) {
  return (
    <div className="relative h-full w-full">
      <FacilityMapView
        id={widget.facilityId}
        fill
        controls={!widget.hideControls}
        initialAtc={!!widget.atc}
        initialRoutes={!!widget.routes}
      />
      {editing && (
        <label className="absolute right-2 top-2 z-[50] flex items-center gap-1.5 rounded-md border bg-background/95 px-2.5 py-1.5 text-xs font-medium shadow-lg backdrop-blur">
          <input
            type="checkbox"
            checked={!widget.hideControls}
            onChange={(e) => onChange(widget.id, { hideControls: !e.target.checked })}
          />
          Show controls
        </label>
      )}
    </div>
  );
}
