import * as React from "react";

import {notifyDesktop} from "@/lib/desktop-notify";
import {useSetting} from "@/lib/settings";

/**
 * Native notifications for published restrictions (#348).
 *
 * Deliberately *not* a second detector. `RestrictionAlerts` already works out which restrictions
 * are genuinely new — it skips the set present at first load, forgets keys that drop off so a
 * cancel-then-reissue fires again, and stays quiet during historical replay. Growing a parallel
 * copy of that logic would mean the toast and the notification could disagree about what happened.
 * Instead that component calls this with the alerts it just raised.
 */

/** The identity-key prefix each alert kind uses, mapped to the tab that shows it. */
const TAB_BY_PREFIX: Record<string, string> = {
  gs: "ground-stops",
  gdp: "gdp",
  tmi: "restrictions",
  prog: "programs",
};

/** The alert shape `restriction-alerts.tsx` builds. Kept structural to avoid exporting its internals. */
export type NotifiableAlert = {
  key: string;
  kind: string;
  title: string;
  lines: string[];
};

function routeFor(key: string): string {
  const tab = TAB_BY_PREFIX[key.split(":")[0] ?? ""] ?? "restrictions";
  return `/ops/tmu?tab=${tab}`;
}

/**
 * Returns a function to hand newly-raised restriction alerts to.
 *
 * A no-op unless the user opted in; `notifyDesktop` separately refuses on the web build, so this is
 * inert there however it is called.
 */
export function useRestrictionNotifier(): (alerts: NotifiableAlert[]) => void {
  const {value: enabled} = useSetting<boolean>("notifications.restrictions", false);
  const {value: soundOn} = useSetting<boolean>("sounds.restrictions", false);
  const {value: volume} = useSetting<string>("sounds.restrictions.volume", "normal");

  return React.useCallback(
    (alerts: NotifiableAlert[]) => {
      for (const alert of alerts) {
        void notifyDesktop(
          {
            category: "restrictions",
            // "Ground Stop: KATL" — the kind leads, because that is what decides whether this is
            // worth putting down what you are doing for.
            title: `${alert.kind}: ${alert.title}`,
            body: alert.lines.join(" · "),
            route: routeFor(alert.key),
          },
          enabled,
          {enabled: soundOn, volume},
        );
      }
    },
    [enabled, soundOn, volume],
  );
}
