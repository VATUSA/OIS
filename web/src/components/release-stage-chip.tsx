import {StatusPill, cn} from "@ois/ui";

import {BUILD_STAGE} from "@/lib/build-stage";
import {type Stage, stageLabel, stageTitle, stageTone} from "@/lib/stage";

/**
 * The release-stage chip ("ALPHA", "BETA", "RC") shown beside the OIS name and the footer version
 * (#841). Renders nothing for stable. `compact` fits the 60px collapsed sidebar rail.
 */
export function ReleaseStageChip({
  stage = BUILD_STAGE,
  compact = false,
}: {
  stage?: Stage;
  compact?: boolean;
}) {
  if (stage === "stable") return null;
  return (
    <span title={stageTitle(stage)} data-release-stage={stage} className="inline-flex">
      <StatusPill
        tone={stageTone(stage)}
        className={cn("uppercase tracking-[0.06em]", compact && "px-1.5 py-0 text-[10px] leading-4")}
      >
        {stageLabel(stage)}
      </StatusPill>
    </span>
  );
}
