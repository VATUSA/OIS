export interface ChangelogEntry {
  /** Stable and ordered with the array — e.g. an ISO date + slug. */
  id: string;
  /** Display date, e.g. "2026-09-14". */
  date: string;
  title: string;
  highlights: string[];
}

/**
 * Newest first. Add an entry here and rebuild to publish release notes — nothing else to touch
 * (see the "What's new" panel, `components/whats-new.tsx`).
 */
export const CHANGELOG: ChangelogEntry[] = [
  {
    id: "2026-09-14-whats-new",
    date: "2026-09-14",
    title: "What's new panel",
    highlights: ["You'll see a summary here whenever OIS ships something worth knowing about."],
  },
];

/**
 * Entries newer than `lastSeenId`, newest first. If `lastSeenId` is missing or isn't found in the
 * current changelog (stale — older than everything still in the module), every entry counts as
 * unseen rather than showing nothing.
 */
export function unseenEntries(
  entries: ChangelogEntry[],
  lastSeenId: string | undefined,
): ChangelogEntry[] {
  const seenIdx = entries.findIndex((e) => e.id === lastSeenId);
  return seenIdx === -1 ? entries : entries.slice(0, seenIdx);
}
