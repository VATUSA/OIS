import {STAT_METRICS, StatWidgetView} from "./stat-widgets";
import {VIEW_OPTIONS, ViewWidgetView} from "./view-widgets";
import type {Widget} from "./types";

/** Renders a widget's body (no chrome — that's WidgetFrame). */
export function WidgetBody({ widget }: { widget: Widget }) {
  switch (widget.kind) {
    case "stat":
      return <StatWidgetView metric={widget.metric} />;
    case "view":
      return <ViewWidgetView view={widget.view} icao={widget.icao} />;
    default:
      return (
        <div className="p-4 text-sm text-muted-foreground">
          This widget type isn&apos;t available yet.
        </div>
      );
  }
}

/** The header label for a widget — the user's override, else a sensible default. */
export function widgetTitle(widget: Widget): string {
  if (widget.title) return widget.title;
  switch (widget.kind) {
    case "stat":
      return STAT_METRICS.find((m) => m.id === widget.metric)?.label ?? "Stat";
    case "view": {
      const label = VIEW_OPTIONS.find((v) => v.id === widget.view)?.label ?? widget.view;
      return `${widget.icao} · ${label}`;
    }
    case "map":
      return "Map";
    default:
      return "Widget";
  }
}
