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
    id: "2026-10-01-advisories-and-departure-runways",
    date: "2026-10-01",
    title: "Advisories, departure runways, and a national view",
    highlights: [
      "You can now write ADVZY advisories in OIS — Reroute, Ground Delay Program and Ground Stop — and publish them to their own Discord channel. Cancel one and a correction is posted beneath it rather than the original being rewritten.",
      "Publishing a GDP or Ground Stop writes its advisory for you, numbered per facility.",
      "TMI posts are proper NTML lines now, carrying the log time, valid window and requesting/providing facilities. Editing a published TMI posts a corrected row, and cancellations post too.",
      "IDST predicts a departure runway for parked and prefiled flights, your ARTCC can declare the gate and SID rules behind it, and the assigned runway feeds taxi and ETE estimates instead of being guessed from heading.",
      "Two held departures can swap release times without re-metering.",
      "A new National (NAS) board ranks airports by demand against capacity across the country, and a dashboard widget can be scoped to the NAS rather than one facility.",
      "Gate data now covers every airport instead of one, so taxi estimates are far closer to reality, and the surface map shows stand detail.",
      "Event movement counts come from observed departures and arrivals over the event's own window, so they are no longer inflated by filed flight plans that never flew.",
      "A saved replay can be deleted once you are done with it, releasing the stored positions it was holding.",
      "Admins can create a service account and give it roles from the UI.",
      "Map fixes: the bundled ARTCC boundaries are repaired, an ATC pill wins its own hover, and a centre nobody is working is no longer shaded.",
      "On the desktop app, macOS windows use the real traffic lights and Windows windows keep their controls everywhere — including before you sign in.",
    ],
  },
  {
    id: "2026-09-16-console-redesign",
    date: "2026-09-16",
    title: "A redesigned OIS",
    highlights: [
      "OIS has a new look: a dark, focused operator console that's consistent across every page.",
      "Navigation is simpler — Advisories and Operations live in the top bar, with Planning, Historical, and Admin tools gathered under a single Admin workspace.",
      "You only see links to the pages you have access to.",
      "Tables now load faster with a compact default view you can expand, and paging through long lists.",
    ],
  },
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
