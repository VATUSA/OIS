import {Modal} from "@ois/ui";
import {useEffect, useRef, useState} from "react";

import {useMe} from "@/lib/auth";
import {
  CHANGELOG,
  panelSize,
  shotColumns,
  shouldSeed,
  unseenEntries,
  type ChangelogEntry,
  type Screenshot,
} from "@/lib/changelog";
import {usePreferences, useSavePreferences} from "@/lib/preferences";

const NAMESPACE = "changelog";
const OPEN_EVENT = "ois:whats-new";

interface ChangelogPrefs {
  lastSeenId?: string;
}

/**
 * Reopens the panel with the whole changelog, whatever the user has already seen (#665). It doesn't
 * move `lastSeenId` — reading old notes again isn't the same as acknowledging new ones.
 */
export const openWhatsNew = () => window.dispatchEvent(new Event(OPEN_EVENT));

/**
 * "What's new" panel: shows a signed-in user any changelog entries newer than the last one they
 * dismissed, once per new entry, and the whole changelog on demand via {@link openWhatsNew}.
 * Mounted once in the root layout; gated on `useMe()` here so a signed-out visitor never triggers
 * the preferences query at all.
 */
export function WhatsNew() {
  const { data: me } = useMe();
  if (!me) return null;
  return <WhatsNewInner />;
}

function WhatsNewInner() {
  const prefs = usePreferences<ChangelogPrefs>(NAMESPACE);
  const save = useSavePreferences<ChangelogPrefs>(NAMESPACE);
  // `unseen` is the once-per-release prompt; `all` is the user asking to read the changelog again.
  const [mode, setMode] = useState<"closed" | "unseen" | "all">("closed");
  // Runs the seed-or-show decision exactly once per mount, once the preference query succeeds —
  // never flashes the panel while loading, never re-seeds over a failed load, and never re-opens it
  // if `prefs.data` later refetches.
  const decided = useRef(false);

  const newestId = CHANGELOG[0]?.id;

  useEffect(() => {
    if (!prefs.isSuccess || decided.current || !newestId) return;
    decided.current = true;
    if (shouldSeed(prefs.data)) {
      // Brand-new user — seed to newest, don't show a backlog.
      save.mutate({ lastSeenId: newestId });
      return;
    }
    if (unseenEntries(CHANGELOG, prefs.data?.lastSeenId).length > 0) {
      setMode("unseen");
    }
    // `save` is listed for honesty, not for effect: the `decided` latch above makes every re-run a
    // no-op, so a fresh mutation identity cannot seed twice (#329).
  }, [prefs.isSuccess, prefs.data, newestId, save]);

  useEffect(() => {
    const open = () => setMode("all");
    window.addEventListener(OPEN_EVENT, open);
    return () => window.removeEventListener(OPEN_EVENT, open);
  }, []);

  const dismiss = () => {
    if (mode === "unseen" && newestId) save.mutate({ lastSeenId: newestId });
    setMode("closed");
  };

  if (mode === "closed" || !newestId) return null;
  const entries = mode === "all" ? CHANGELOG : unseenEntries(CHANGELOG, prefs.data?.lastSeenId);

  return (
    <Modal open onClose={dismiss} title="What's new" size={panelSize(entries)}>
      <ChangelogList entries={entries} />
    </Modal>
  );
}

/** The entries, each split into its sections — heading, bullets, then any screenshots. */
export function ChangelogList({ entries }: { entries: ChangelogEntry[] }) {
  return (
    <div className="flex flex-col divide-y divide-line-soft [&>*]:py-3 [&>*:first-child]:pt-0 [&>*:last-child]:pb-0">
      {entries.map((entry) => (
        <article key={entry.id} className="flex flex-col gap-1">
          <div className="flex items-baseline justify-between gap-2">
            <span className="font-semibold text-ink">{entry.title}</span>
            <span className="shrink-0 font-mono text-xs text-ink-3">{entry.date}</span>
          </div>
          {entry.sections.map((section, i) => (
            <section key={i} className="flex flex-col gap-1 [&+&]:mt-2">
              {section.heading && <h3 className="text-sm font-semibold text-ink">{section.heading}</h3>}
              {section.highlights.length > 0 && (
                <ul className="list-disc pl-5 text-sm text-ink-2 marker:text-ink-3">
                  {section.highlights.map((h, j) => (
                    <li key={j}>{h}</li>
                  ))}
                </ul>
              )}
              {section.shots && section.shots.length > 0 && <ShotGrid shots={section.shots} />}
            </section>
          ))}
        </article>
      ))}
    </div>
  );
}

/** Literal classes per column count, so Tailwind sees every one. Always one column on a narrow screen. */
const GRID_COLUMNS = {
  1: "grid-cols-1",
  2: "grid-cols-1 sm:grid-cols-2",
  3: "grid-cols-1 sm:grid-cols-3",
} as const;

/**
 * A section's screenshots, laid out by how many there are (see `shotColumns`). Thumbnails share one
 * top-anchored box so the grid stays even; every one opens full-size, since at three columns nothing
 * in an OIS screenshot is legible.
 */
export function ShotGrid({ shots }: { shots: Screenshot[] }) {
  const [enlarged, setEnlarged] = useState<Screenshot | null>(null);
  return (
    <>
      <div className={`mt-2 grid gap-3 ${GRID_COLUMNS[shotColumns(shots.length)]}`} data-testid="shot-grid">
        {shots.map((shot) => (
          <figure key={shot.src} className="flex flex-col gap-1">
            <button
              type="button"
              onClick={() => setEnlarged(shot)}
              aria-label={`Enlarge: ${shot.alt}`}
              className="overflow-hidden rounded-md border border-line bg-card outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <img src={shot.src} alt={shot.alt} className="aspect-shot w-full object-cover object-top" />
            </button>
            {shot.caption && <figcaption className="text-xs text-ink-3">{shot.caption}</figcaption>}
          </figure>
        ))}
      </div>
      <Modal open={enlarged != null} onClose={() => setEnlarged(null)} title={enlarged?.caption ?? enlarged?.alt} size="xl">
        {enlarged && <img src={enlarged.src} alt={enlarged.alt} className="h-auto w-full rounded-md" />}
      </Modal>
    </>
  );
}
