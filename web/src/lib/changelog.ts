/**
 * A screenshot shown with a changelog section. Import the file from `@/assets/changelog/…` so Vite
 * hashes it and it is served from our own origin — the desktop app's CSP allows `'self'` images and
 * nothing remote (#429, #665).
 */
export interface Screenshot {
  src: string;
  /** Required: describes the change the shot demonstrates, not the UI chrome. */
  alt: string;
  /** Optional one-line caption under the frame. */
  caption?: string;
}

export interface ChangelogSection {
  /** Omitted for a single unheaded section, which renders exactly like a flat list. */
  heading?: string;
  highlights: string[];
  shots?: Screenshot[];
}

export interface ChangelogEntry {
  /** Stable and ordered with the array — e.g. an ISO date + slug. */
  id: string;
  /** Display date, e.g. "2026-09-14". */
  date: string;
  title: string;
  sections: ChangelogSection[];
}

/** Shots one entry may carry, summed across its sections — bounds what one update can weigh. */
export const MAX_SHOTS_PER_ENTRY = 8;
/** Only the newest this-many entries may carry shots; older ones must drop them so the bundle stays bounded. */
export const SHOT_RETENTION = 3;

/**
 * Newest first. Add an entry here and rebuild to publish release notes — nothing else to touch
 * (see the "What's new" panel, `components/whats-new.tsx`).
 */
export const CHANGELOG: ChangelogEntry[] = [
  {
    id: "2026-10-04-airspace-monitor",
    date: "2026-10-04",
    title: "The Airspace Monitor, live screens, and an API for machines",
    sections: [
      {
        heading: "Airspace Monitor",
        highlights: [
          "OIS now has an Airspace Monitor: one table per ARTCC, a row per high sector, showing peak concurrent traffic in 15-minute Zulu bins against each sector's Monitor Alert Parameter.",
          "A sector turns amber when airborne plus proposed traffic would exceed its MAP, and red when airborne traffic alone already does — the same reading as the FAA's Monitor.",
          "Each sector's MAP is yours to set, facility by facility, and resetting one back to the default clears the override rather than pinning the default in place.",
          "You can consolidate a whole area into one sector from the row menu, and see an ARTCC's first-tier neighbours beneath its own table.",
          "ATC sector volumes — geometry and altitude limits — are held in Postgres and drawn on an admin sector map, with tooltips, a legend and layer toggles.",
          "Live sector staffing comes from vNAS, so a sector shows who is working it.",
        ],
      },
      {
        heading: "Screens that update themselves",
        highlights: [
          "Live views no longer poll. When the traffic feed brings new data, OIS tells the screens that are open, and advisories, the ACE board, runway views and FCA lists refresh as they change.",
          "Nudges reach every replica, so what you see is the same whichever server answered you.",
          "A memory leak that restarted the backend roughly every half hour is fixed.",
        ],
      },
      {
        heading: "For integrators",
        highlights: [
          "Machine credentials can now drive the TMU, flow, GDP and release paths, not just read them, and they can open the realtime socket and subscribe to topics.",
          "A service account can be granted scoped permissions, capped by expiry, and every API caller now has a rate limit you can set per credential — with its request volume visible alongside it.",
          "There is an integrator guide, a generated permission map, and Swagger on the API docs, so an endpoint's credentials and permission are written down rather than discovered.",
          "The API documents exactly which credentials each endpoint accepts, so a key is never offered a path that would refuse it.",
        ],
      },
      {
        heading: "Access and groups",
        highlights: [
          "Groups are managed from the UI — what they grant, who is in them, and which VATUSA staff roles map onto them — with cards and permission domains collapsed until you open them.",
          "One permission resolver now honours scope and deny together, and the editor's no-escalation guard is scope-aware, so you cannot grant past what you hold.",
          "VATUSA role sync pulls the whole division daily, ignores a replayed webhook, says what VATUSA objected to, and records where each grant came from so a later sync can revoke only its own rows.",
          "A hand-edited user can be taken off role sync, with that fact shown and a resync available.",
        ],
      },
      {
        heading: "Flow, TMU and maps",
        highlights: [
          "A TGUI arrival ladder shows ETA and STA on opposing rails.",
          "Two held IDST departures can swap release times when they share an airport and runway.",
          "FCA colours are named swatches with a custom pick, and FCA create, edit, reorder and delete are gated on the FCA's own ARTCC.",
          "An advisory can be planned inside an event package, and deactivating a package posts cancellations for what it ends early.",
          "Map fixes: hover cards render on screen and the nearer of an aircraft or ATC pill wins, the surface viewer centres on the airport, X-Plane stand detail is kept, and Honolulu and San Francisco Oceanic resolve correctly.",
        ],
      },
      {
        heading: "Desktop app",
        highlights: [
          "The desktop app writes a redacted log file and can send diagnostics to staff when something goes wrong.",
          "macOS no longer re-prompts for the keychain, sign-in has its own OIS-styled tab, and a cached release is served when the GitHub refresh fails.",
        ],
      },
    ],
  },
  {
    id: "2026-10-01-advisories-and-departure-runways",
    date: "2026-10-01",
    title: "Advisories, departure runways, and a national view",
    sections: [
      {
        heading: "Advisories and TMIs",
        highlights: [
          "You can now write ADVZY advisories in OIS — Reroute, Ground Delay Program and Ground Stop — and publish them to their own Discord channel. Cancel one and a correction is posted beneath it rather than the original being rewritten.",
          "Publishing a GDP or Ground Stop writes its advisory for you, numbered per facility.",
          "TMI posts are proper NTML lines now, carrying the log time, valid window and requesting/providing facilities. Editing a published TMI posts a corrected row, and cancellations post too.",
        ],
      },
      {
        heading: "Departures and taxi",
        highlights: [
          "IDST predicts a departure runway for parked and prefiled flights, your ARTCC can declare the gate and SID rules behind it, and the assigned runway feeds taxi and ETE estimates instead of being guessed from heading.",
          "Two held departures can swap release times without re-metering.",
          "Gate data now covers every airport instead of one, so taxi estimates are far closer to reality, and the surface map shows stand detail.",
        ],
      },
      {
        heading: "National view",
        highlights: [
          "A new National (NAS) board ranks airports by demand against capacity across the country, and a dashboard widget can be scoped to the NAS rather than one facility.",
        ],
      },
      {
        heading: "Events and replays",
        highlights: [
          "Event movement counts come from observed departures and arrivals over the event's own window, so they are no longer inflated by filed flight plans that never flew.",
          "A saved replay can be deleted once you are done with it, releasing the stored positions it was holding.",
        ],
      },
      {
        heading: "Admin",
        highlights: [
          "Admins can create a service account and give it roles from the UI.",
        ],
      },
      {
        heading: "Map and desktop",
        highlights: [
          "Map fixes: the bundled ARTCC boundaries are repaired, an ATC pill wins its own hover, and a centre nobody is working is no longer shaded.",
          "On the desktop app, macOS windows use the real traffic lights and Windows windows keep their controls everywhere — including before you sign in.",
        ],
      },
    ],
  },
  {
    id: "2026-09-16-console-redesign",
    date: "2026-09-16",
    title: "A redesigned OIS",
    sections: [
      {
        highlights: [
          "OIS has a new look: a dark, focused operator console that's consistent across every page.",
          "Navigation is simpler — Advisories and Operations live in the top bar, with Planning, Historical, and Admin tools gathered under a single Admin workspace.",
          "You only see links to the pages you have access to.",
          "Tables now load faster with a compact default view you can expand, and paging through long lists.",
        ],
      },
    ],
  },
  {
    id: "2026-09-14-whats-new",
    date: "2026-09-14",
    title: "What's new panel",
    sections: [
      {
        highlights: [
          "You'll see a summary here whenever OIS ships something worth knowing about.",
        ],
      },
    ],
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

/** Every shot in an entry, across its sections. */
export function entryShots(entry: ChangelogEntry): Screenshot[] {
  return entry.sections.flatMap((s) => s.shots ?? []);
}

/**
 * Grid columns for a section's shots: one shot spans the panel, two to four sit in pairs, five or
 * more in threes. Capped at 3 so a thumbnail stays readable in the ~856px panel body.
 */
export function shotColumns(count: number): 1 | 2 | 3 {
  if (count <= 1) return 1;
  if (count <= 4) return 2;
  return 3;
}

/** The panel widens only when something it shows has shots; text-only entries keep today's width. */
export function panelSize(entries: ChangelogEntry[]): "md" | "xl" {
  return entries.some((e) => entryShots(e).length > 0) ? "xl" : "md";
}

/**
 * Authoring rules for the changelog, as human-readable problems (empty = valid): the per-entry shot
 * cap, a non-blank `alt` on every shot, retention (only the newest {@link SHOT_RETENTION} entries
 * carry shots), and no remote `src` — a remote image is blocked by the desktop app's CSP.
 */
export function changelogProblems(entries: ChangelogEntry[]): string[] {
  const problems: string[] = [];
  entries.forEach((entry, index) => {
    const shots = entryShots(entry);
    if (shots.length > MAX_SHOTS_PER_ENTRY) {
      problems.push(`${entry.id}: ${shots.length} shots (max ${MAX_SHOTS_PER_ENTRY})`);
    }
    if (shots.length > 0 && index >= SHOT_RETENTION) {
      problems.push(`${entry.id}: only the newest ${SHOT_RETENTION} entries may carry shots`);
    }
    for (const shot of shots) {
      if (!shot.alt.trim()) problems.push(`${entry.id}: a shot has no alt text (${shot.src})`);
      if (/^(https?:)?\/\//i.test(shot.src)) {
        problems.push(`${entry.id}: ${shot.src} is remote — import it from @/assets/changelog/`);
      }
    }
  });
  return problems;
}
