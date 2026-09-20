import * as React from "react";
import {useQuery} from "@tanstack/react-query";

import {ois} from "@/lib/api";
import {useMe} from "@/lib/auth";
import {notifyDesktop, type NotifyCategory} from "@/lib/desktop-notify";
import {useFcas, useFcaTraffic} from "@/lib/fca";
import {can} from "@/lib/platform";
import {useSetting} from "@/lib/settings";

/**
 * The notification detectors that don't already have one (#348).
 *
 * Restrictions are handled elsewhere — `restriction-alerts.tsx` already works out which are new, so
 * that detection is teed off rather than duplicated (`lib/notify-restrictions.ts`). What's left has
 * no existing detector, and each one here follows the same shape that file proved:
 *
 *   first load seeds silently → later arrivals fire → keys that drop off are forgotten,
 *   so the same thing recurring notifies again.
 *
 * All of it is inert on the web build: `can("notifications")` is false there, every setting is
 * hidden, and `notifyDesktop` refuses regardless.
 */

/**
 * Fires `notify` for keys that appear after the first settled load.
 *
 * The `null` seed is the important part — without it, every notification category would dump its
 * entire current state at the user the moment the app opens.
 */
function useNewKeys(
  entries: Map<string, {title: string; body: string; route: string}>,
  settled: boolean,
  category: NotifyCategory,
  enabled: boolean,
) {
  const known = React.useRef<Set<string> | null>(null);

  React.useEffect(() => {
    if (known.current == null) {
      if (settled) known.current = new Set(entries.keys());
      return;
    }
    for (const [key, entry] of entries) {
      if (!known.current.has(key)) {
        known.current.add(key);
        void notifyDesktop({category, ...entry}, enabled);
      }
    }
    for (const key of [...known.current]) if (!entries.has(key)) known.current.delete(key);
  }, [entries, settled, category, enabled]);
}

/** EDCT releases and heavy metering delay, for one FCA. Both read the same traffic list. */
function FcaNotifier({fcaId, name}: {fcaId: string; name: string}) {
  const {value: releasesOn} = useSetting<boolean>("notifications.releases", false);
  const {value: meteringOn} = useSetting<boolean>("notifications.metering", false);
  const {value: thresholdRaw} = useSetting<string>("notifications.meteringDelayMin", "15");
  const threshold = Number(thresholdRaw) || 15;

  const traffic = useFcaTraffic(fcaId);
  const flights = React.useMemo(() => traffic.data ?? [], [traffic.data]);
  const route = `/ops/fca?fca=${encodeURIComponent(fcaId)}`;

  const released = React.useMemo(() => {
    const m = new Map<string, {title: string; body: string; route: string}>();
    for (const f of flights) {
      // Keyed on the EDCT itself, so a *re-issued* time is a new notification rather than silence.
      if (f.edct) {
        m.set(`${f.callsign}:${f.edct}`, {
          title: `Release: ${f.callsign}`,
          body: `EDCT ${f.edct}z · ${name}`,
          route,
        });
      }
    }
    return m;
  }, [flights, name, route]);

  const delayed = React.useMemo(() => {
    const m = new Map<string, {title: string; body: string; route: string}>();
    for (const f of flights) {
      const delay = f.delay_min ?? 0;
      // Bucketed by threshold crossing, not by exact minute — otherwise one aircraft's delay
      // drifting from 16 to 17 minutes would notify all over again.
      if (delay >= threshold) {
        m.set(`${f.callsign}:${name}`, {
          title: `Metering delay: ${f.callsign}`,
          body: `${Math.round(delay)} min delay crossing ${name}`,
          route,
        });
      }
    }
    return m;
  }, [flights, name, route, threshold]);

  useNewKeys(released, !traffic.isPending, "releases", releasesOn);
  useNewKeys(delayed, !traffic.isPending, "metering", meteringOn);

  return null;
}

/**
 * Notifies on EDCT releases and metering delay across every enabled FCA.
 *
 * Mounts nothing unless one of those categories is actually switched on: each `FcaNotifier` opens a
 * traffic query that polls every 30s, so mounting them regardless would put one request per FCA per
 * 30s on every desktop client — including everyone who never asked for these notifications.
 */
function FcaNotifiers() {
  const {value: releasesOn} = useSetting<boolean>("notifications.releases", false);
  const {value: meteringOn} = useSetting<boolean>("notifications.metering", false);
  // The list query is shared cache with the rest of the app, so it costs nothing extra; the early
  // return below is what stops the per-FCA traffic polls from being opened.
  const fcas = useFcas();

  if (!releasesOn && !meteringOn) return null;

  return (
    <>
      {(fcas.data ?? [])
        .filter((f) => f.enabled)
        .map((f) => (
          <FcaNotifier key={f.id} fcaId={f.id} name={f.name} />
        ))}
    </>
  );
}

/**
 * Flattens `/me`'s nested permission tree into dotted names (`flow.fca.update`).
 *
 * The tree is objects all the way down to an array of actions at each leaf, which is the shape
 * `hasPermission` walks. Flattening lets the diff work on a plain set, so only *additions* notify —
 * a revoke silently drops out rather than announcing itself as a grant.
 */
export function flattenPermissions(node: unknown, prefix = ""): string[] {
  if (Array.isArray(node)) return node.map((action) => `${prefix}.${String(action)}`);
  if (!node || typeof node !== "object") return [];

  return Object.entries(node as Record<string, unknown>).flatMap(([segment, child]) =>
    flattenPermissions(child, prefix ? `${prefix}.${segment}` : segment),
  );
}

/**
 * Notifies when the signed-in user gains a permission or role.
 *
 * `access.granted` invalidates `["me"]`, so this just watches the refetched profile. Comparing
 * against the previous value is what makes it "granted" rather than "you have access" — the nudge
 * itself is broadcast to everyone and says nothing about who changed.
 */
function AccessNotifier() {
  const {value: enabled} = useSetting<boolean>("notifications.access", false);
  const {data: me} = useMe();

  const held = React.useMemo(() => {
    const m = new Map<string, {title: string; body: string; route: string}>();
    for (const role of me?.role_names ?? []) {
      m.set(`role:${role}`, {
        title: "Access granted",
        body: `You were given the ${role} role.`,
        route: "/profile",
      });
    }
    // Permissions as well as roles — the setting promises both, and most grants are permissions
    // rather than a whole role.
    for (const permission of flattenPermissions(me?.permissions)) {
      m.set(`perm:${permission}`, {
        title: "Access granted",
        body: `You were given ${permission}.`,
        route: "/profile",
      });
    }
    return m;
  }, [me?.role_names, me?.permissions]);

  useNewKeys(held, !!me, "access", enabled);
  return null;
}

/**
 * Notifies 24h and 6h before an event the user has claimed an ACE position for.
 *
 * Mirrors the Discord reminder the scheduler already sends — the backend nudges `events.reminder`
 * when it enqueues those DMs, this refetches the user's own claims and works out which one is due.
 * The window check is client-side because the nudge carries no payload.
 */
const REMINDER_TIERS = [
  {hours: 6, label: "6 hours"},
  {hours: 24, label: "24 hours"},
];

function EventReminderNotifier() {
  const {value: enabled} = useSetting<boolean>("notifications.eventReminders", false);
  const {data: me} = useMe();

  const claims = useQuery({
    queryKey: ["my-ace-claims"],
    enabled: !!me && can("notifications"),
    queryFn: async () => {
      const {data} = await ois.GET("/api/v1/me/ace-claims");
      return data ?? [];
    },
  });

  const due = React.useMemo(() => {
    const m = new Map<string, {title: string; body: string; route: string}>();
    const now = Date.now();
    for (const claim of claims.data ?? []) {
      const startsIn = new Date(claim.start_time).getTime() - now;
      if (startsIn <= 0) continue;
      const tier = REMINDER_TIERS.find((t) => startsIn <= t.hours * 3_600_000);
      if (!tier) continue;
      // Keyed by tier as well as claim, so the 24h and 6h reminders are two notifications, and
      // neither repeats.
      m.set(`${claim.claim_id}:${tier.hours}`, {
        title: `${claim.event_title} in ${tier.label}`,
        body: `You're claimed for ${claim.position}.`,
        route: `/planning/events/${claim.event_id}`,
      });
    }
    return m;
  }, [claims.data]);

  useNewKeys(due, !claims.isPending, "eventReminders", enabled);
  return null;
}

/**
 * Mounts every detector that isn't already covered by an existing alert surface.
 *
 * Renders nothing, and mounts nothing at all unless the platform can notify — so the web build
 * doesn't even open the extra queries these detectors need.
 */
export function DesktopNotifiers() {
  const {data: me} = useMe();
  if (!can("notifications") || !me) return null;

  return (
    <>
      <FcaNotifiers />
      <AccessNotifier />
      <EventReminderNotifier />
    </>
  );
}
