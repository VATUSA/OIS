import {FacilityMapView} from "@/pages/facility-map";
import type {FacilityMapWidget} from "./types";

/** Dashboard widget: a per-facility TMU map with color-coded aircraft, rendered in minimal chrome
 * (map + legend), with the ATC/routes overlays fixed by the widget config. */
export function FacilityMapWidgetView({ widget }: { widget: FacilityMapWidget }) {
  return (
    <FacilityMapView
      id={widget.facilityId}
      embed
      initialAtc={!!widget.atc}
      initialRoutes={!!widget.routes}
    />
  );
}
