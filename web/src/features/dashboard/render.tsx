import {ChartWidget} from "./chart-widget";
import {DividerWidgetView, TextWidgetView} from "./layout-widgets";
import {MapWidgetView} from "./map-widget";
import {DATA_SOURCES_BY_ID} from "./sources";
import {STAT_METRICS, StatWidgetView} from "./stat-widgets";
import {TableWidget} from "./table-widget";
import type {Widget} from "./types";
import {VIEW_OPTIONS, ViewWidgetView} from "./view-widgets";

/** Renders a widget's body (no chrome — that's WidgetFrame). */
export function WidgetBody({
  widget,
  editing,
  onUpdate,
}: {
  widget: Widget;
  editing: boolean;
  onUpdate: (id: string, patch: Record<string, unknown>) => void;
}) {
  switch (widget.kind) {
    case "stat":
      return <StatWidgetView metric={widget.metric} />;
    case "view":
      return <ViewWidgetView widget={widget} editing={editing} onChange={onUpdate} />;
    case "map":
      return <MapWidgetView initialFlight={widget.initialFlight} widgetId={widget.id} />;
    case "table":
      return <TableWidget widget={widget} editing={editing} onChange={onUpdate} />;
    case "chart":
      return <ChartWidget widget={widget} editing={editing} onChange={onUpdate} />;
    case "text":
      return <TextWidgetView widget={widget} editing={editing} onChange={onUpdate} />;
    case "divider":
      return <DividerWidgetView widget={widget} editing={editing} onChange={onUpdate} />;
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
  if ("title" in widget && widget.title) return widget.title;
  switch (widget.kind) {
    case "stat":
      return STAT_METRICS.find((m) => m.id === widget.metric)?.label ?? "Stat";
    case "view": {
      const label = VIEW_OPTIONS.find((v) => v.id === widget.view)?.label ?? widget.view;
      return `${widget.icao} · ${label}`;
    }
    case "map":
      return "Map";
    case "table": {
      const label = DATA_SOURCES_BY_ID[widget.source]?.label ?? "Table";
      return widget.params?.icao ? `${widget.params.icao} · ${label}` : label;
    }
    case "chart": {
      const label = DATA_SOURCES_BY_ID[widget.source]?.label ?? "Chart";
      return widget.params?.icao ? `${widget.params.icao} · ${label}` : label;
    }
    case "text":
    case "divider":
      return ""; // bare widgets render no header
    default:
      return "Widget";
  }
}
