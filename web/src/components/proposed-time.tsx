import { hhmmZulu } from "@/lib/time";

/** What a proposed time rests on, for its tooltip. */
export const PROPOSED_TITLE =
  "Proposed: timed from the filed departure time plus 10 min, and no sooner than now + 10 min. No ready time, release, CFR or EDCT yet.";

/**
 * A Zulu time, marked when it rests on the flight's proposed wheels-up (#814): the backend's
 * `wheels_up_proposed`, a ground flight with no ready time, release, CFR or GDP EDCT. The marker is a
 * letter as well as a color, so it reads without the color.
 */
export function ProposedTime({
  iso,
  proposed,
  className,
}: {
  iso: string | null | undefined;
  proposed: boolean;
  className?: string;
}) {
  const time = hhmmZulu(iso);
  if (!proposed || time === "—") return <span className={className}>{time}</span>;
  return (
    <span className={className} title={PROPOSED_TITLE}>
      {time}
      <span aria-hidden="true" className="ml-1 font-semibold text-flight-proposed">
        P
      </span>
      <span className="sr-only"> proposed</span>
    </span>
  );
}
