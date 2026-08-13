import {useCallback, useEffect, useRef} from "react";
import {useToast} from "@ois/ui";

import {useMe} from "@/lib/auth";
import {feedIsStale, useFeedStatus} from "@/lib/feed";
import {hasPermission} from "@/lib/permissions";

/**
 * The VATSIM feed is considered stale after this long without a fresh ingest.
 * The poller runs every 15s and the client polls status every 30s, so a healthy
 * feed's observed age tops out around ~45s — 90s leaves comfortable margin.
 */
const STALE_MS = 90_000;

/**
 * Headless watcher: warns once (via toast) when the VATSIM feed goes stale or
 * unhealthy, and again when it recovers. Mounted in the root layout so the
 * warning follows the user across every page. Only active for controllers who
 * actually consume the feed.
 */
export function FeedWatcher() {
  const { data: me } = useMe();
  const enabled = hasPermission(me, "tmu.program.read");
  const { data } = useFeedStatus();
  const toast = useToast();
  const wasStale = useRef(false);

  const check = useCallback(() => {
    if (!enabled || !data) return;
    const stale = feedIsStale(data, Date.now(), STALE_MS);

    if (stale && !wasStale.current) {
      toast.warning("VATSIM feed is stale", {
        description: data.last_error
          ? `Feed error: ${data.last_error}. Traffic numbers may be out of date.`
          : "No fresh data from VATSIM — traffic numbers may be out of date until it recovers.",
        duration: 8000,
      });
    } else if (!stale && wasStale.current) {
      toast.success("VATSIM feed recovered", {
        description: "Live traffic is up to date again.",
      });
    }
    wasStale.current = stale;
  }, [enabled, data, toast]);

  useEffect(() => {
    check();
    const id = window.setInterval(check, 15_000);
    return () => window.clearInterval(id);
  }, [check]);

  return null;
}
