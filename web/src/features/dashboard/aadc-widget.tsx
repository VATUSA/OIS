import {AadcChart} from "@/components/aadc-chart";
import {useAadc} from "@/lib/aadc";

import type {AadcWidget} from "./types";
import {useReportWidgetStatus} from "./widget-status";

/** Dashboard rendering of the AADC chart (#242) — a thin wrapper over the shared `AadcChart`
 * component, persisting the bucket/dimension choice on the widget instance itself. */
export function AadcWidgetView({
  widget,
  onUpdate,
}: {
  widget: AadcWidget;
  onUpdate: (id: string, patch: Record<string, unknown>) => void;
}) {
  const aadc = useAadc(widget.icao, widget.bucketMin);
  useReportWidgetStatus(aadc.isFetching, aadc.dataUpdatedAt, aadc.refetch);

  if (aadc.isError) {
    return (
      <div className="p-4 text-sm text-ink-3">
        Couldn&apos;t load AADC data for <span className="font-mono">{widget.icao}</span>.
      </div>
    );
  }
  if (!aadc.data) {
    return <div className="p-4 text-sm text-ink-3">Loading <span className="font-mono">{widget.icao}</span>…</div>;
  }

  return (
    <div className="p-3">
      <AadcChart
        icao={widget.icao}
        bucketMin={widget.bucketMin}
        onBucketMinChange={(v) => onUpdate(widget.id, { bucketMin: v })}
        dimension={widget.dimension}
        onDimensionChange={(v) => onUpdate(widget.id, { dimension: v })}
        buckets={aadc.data.buckets}
        aar={aadc.data.aar}
        isLoading={aadc.isFetching}
      />
    </div>
  );
}
