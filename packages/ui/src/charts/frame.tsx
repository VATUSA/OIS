import * as React from "react";
import {Chart} from "@tanstack/react-charts/tooltip";

import {cn} from "../lib/utils";
import {useElementSize} from "./use-size";

type AnyDefinition = React.ComponentProps<typeof Chart>["definition"];
type RenderTooltip = React.ComponentProps<typeof Chart>["renderTooltipBody"];

/**
 * Sizes a chart to its container (fixed `height`, or fill the parent with `height="fill"`) and mounts
 * it with the shared tooltip plumbing.
 */
export function ChartFrame({
  definition,
  label,
  height,
  renderTooltip,
  className,
}: {
  definition: AnyDefinition;
  label: string;
  height: number | "fill";
  renderTooltip?: RenderTooltip;
  className?: string;
}) {
  const [ref, size] = useElementSize<HTMLDivElement>();
  const h = height === "fill" ? size.h : height;
  return (
    <div ref={ref} className={cn("min-w-0", height === "fill" && "size-full min-h-0", className)} style={height === "fill" ? undefined : { height }}>
      {size.w > 0 && h > 0 && (
        <Chart definition={definition} ariaLabel={label} width={size.w} height={h} renderTooltipBody={renderTooltip} />
      )}
    </div>
  );
}
