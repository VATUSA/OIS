import {Modal} from "@ois/ui";
import {useEffect, useRef, useState} from "react";

import {useMe} from "@/lib/auth";
import {CHANGELOG, shouldSeed, unseenEntries} from "@/lib/changelog";
import {usePreferences, useSavePreferences} from "@/lib/preferences";

const NAMESPACE = "changelog";

interface ChangelogPrefs {
  lastSeenId?: string;
}

/**
 * "What's new" panel: shows a signed-in user any changelog entries newer than the last one they
 * dismissed, once per new entry. Mounted once in the root layout; gated on `useMe()` here so a
 * signed-out visitor never triggers the preferences query at all.
 */
export function WhatsNew() {
  const { data: me } = useMe();
  if (!me) return null;
  return <WhatsNewInner />;
}

function WhatsNewInner() {
  const prefs = usePreferences<ChangelogPrefs>(NAMESPACE);
  const save = useSavePreferences<ChangelogPrefs>(NAMESPACE);
  const [open, setOpen] = useState(false);
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
      setOpen(true);
    }
  }, [prefs.isSuccess, prefs.data, newestId]);

  const dismiss = () => {
    setOpen(false);
    if (newestId) save.mutate({ lastSeenId: newestId });
  };

  if (!open || !newestId) return null;
  const entries = unseenEntries(CHANGELOG, prefs.data?.lastSeenId);

  return (
    <Modal open={open} onClose={dismiss} title="What's new">
      <div className="flex flex-col divide-y divide-line-soft [&>*]:py-3 [&>*:first-child]:pt-0 [&>*:last-child]:pb-0">
        {entries.map((entry) => (
          <div key={entry.id} className="flex flex-col gap-1">
            <div className="flex items-baseline justify-between gap-2">
              <span className="font-semibold text-ink">{entry.title}</span>
              <span className="shrink-0 font-mono text-xs text-ink-3">{entry.date}</span>
            </div>
            <ul className="list-disc pl-5 text-sm text-ink-2 marker:text-ink-3">
              {entry.highlights.map((h, i) => (
                <li key={i}>{h}</li>
              ))}
            </ul>
          </div>
        ))}
      </div>
    </Modal>
  );
}
