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

/**
 * Whether this user's changelog prefs should be silently seeded to newest instead of shown a
 * backlog (#206 AC5) — true for a brand-new user (the namespace was never saved) and for one whose
 * saved blob holds no `lastSeenId` yet. `GET /api/v1/me/preferences/{namespace}` returns `{}`, not
 * `null`, for an unset namespace, so this must check the specific field rather than the whole
 * blob's nullness — checking `prefs == null` looks equivalent but never fires and regressed AC5
 * for every user (#206).
 */
export function shouldSeed(prefs: { lastSeenId?: string } | null | undefined): boolean {
  return !prefs?.lastSeenId;
}
