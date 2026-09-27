import * as React from "react";
import {useQuery} from "@tanstack/react-query";

import {ois} from "@/lib/api";
import {useMe, type Me} from "@/lib/auth";
import {notifyDesktop, type NotifyCategory} from "@/lib/desktop-notify";
import {useFcas, useFcaTraffic} from "@/lib/fca";
import {hasPermission} from "@/lib/permissions";
import {can} from "@/lib/platform";
import {hhmmZulu} from "@/lib/time";
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
 *
 * `settled` must mean the first load *succeeded*, not merely that it is no longer pending: an errored
 * query is not pending either, and seeding from its empty data made the first successful poll look
 * like every entry was new — one native notification per flight already holding an EDCT
 * (VATUSA/OIS#348 review). Pass `query.isSuccess`.
 */
type Entry = {title: string; body: string; route: string};

function useNewKeys(
  entries: Map<string, Entry>,
  settled: boolean,
  category: NotifyCategory,
  enabled: boolean,
  /** Fold everything that arrived together into one notification, instead of one per key. */
  collapse?: (fresh: Map<string, Entry>) => Entry,
) {
  const known = React.useRef<Set<string> | null>(null);

  React.useEffect(() => {
    if (known.current == null) {
      if (settled) known.current = new Set(entries.keys());
      return;
    }
    const fresh = new Map<string, Entry>();
    for (const [key, entry] of entries) {
      if (!known.current.has(key)) {
        known.current.add(key);
        fresh.set(key, entry);
      }
    }
    for (const key of [...known.current]) if (!entries.has(key)) known.current.delete(key);

    if (collapse && fresh.size) void notifyDesktop({category, ...collapse(fresh)}, enabled);
    else for (const entry of fresh.values()) void notifyDesktop({category, ...entry}, enabled);
  }, [entries, settled, category, enabled, collapse]);
}

/** EDCT releases and heavy metering delay, for one FCA. Both read the same traffic list. */
export function FcaNotifier({fcaId, name}: {fcaId: string; name: string}) {
  const {value: releasesOn} = useSetting<boolean>("notifications.releases", false);
  const {value: meteringOn} = useSetting<boolean>("notifications.metering", false);
  const {value: thresholdRaw} = useSetting<string>("notifications.meteringDelayMin", "15");
  const threshold = Number(thresholdRaw) || 15;

  // Background polling on: this component exists to notice releases and metering delay while the
  // window is hidden, which is when the default interval would be skipped.
  const traffic = useFcaTraffic(fcaId, false, {background: true});
  const flights = React.useMemo(() => traffic.data ?? [], [traffic.data]);
  const route = `/ops/fca?fca=${encodeURIComponent(fcaId)}`;

  const released = React.useMemo(() => {
    const m = new Map<string, Entry>();
    for (const f of flights) {
      // Keyed on the EDCT itself, so a *re-issued* time is a new notification rather than silence.
      if (f.edct) {
        m.set(`${f.callsign}:${f.edct}`, {
          title: `Release: ${f.callsign}`,
          // `edct` is a full ISO timestamp, not a bare time — interpolated as-is it read
          // "EDCT 2026-09-27T14:05:00Zz" (VATUSA/OIS#348 review).
          body: `EDCT ${hhmmZulu(f.edct)} · ${name}`,
          route,
        });
      }
    }
    return m;
  }, [flights, name, route]);

  const delayed = React.useMemo(() => {
    const m = new Map<string, Entry>();
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

  useNewKeys(released, traffic.isSuccess, "releases", releasesOn);
  useNewKeys(delayed, traffic.isSuccess, "metering", meteringOn);

  return null;
}

/**
 * The ARTCCs whose FCAs this user hears about: their home facility and the ones they visit.
 * `null` means every ARTCC — server admins only.
 *
 * Without this, a ZDC controller who switched releases on was notified of every release at every
 * FCA in the country, and kept one background poll open per FCA (VATUSA/OIS#348 review). Until the
 * VATUSA profile has synced there is no facility to go on, so that is nothing rather than everything.
 */
export function notifyFacilities(me: Me | null | undefined): Set<string> | null {
  if (me?.server_admin) return null;
  const v = me?.vatusa;
  return new Set([...(v?.home_facility ? [v.home_facility] : []), ...(v?.visits ?? [])]);
}

/**
 * Notifies on EDCT releases and metering delay at the FCAs of the user's own ARTCCs.
 *
 * Mounts nothing unless one of those categories is actually switched on: each `FcaNotifier` opens a
 * traffic query that polls every 30s, so mounting them regardless would put one request per FCA per
 * 30s on every desktop client — including everyone who never asked for these notifications.
 */
export function FcaNotifiers() {
  const {value: releasesOn} = useSetting<boolean>("notifications.releases", false);
  const {value: meteringOn} = useSetting<boolean>("notifications.metering", false);
  const {data: me} = useMe();
  // The list query is shared cache with the rest of the app, so it costs nothing extra; the early
  // return below is what stops the per-FCA traffic polls from being opened.
  const fcas = useFcas();

  if (!releasesOn && !meteringOn) return null;

  const facilities = notifyFacilities(me);
  return (
    <>
      {(fcas.data ?? [])
        .filter((f) => f.enabled && (facilities == null || facilities.has(f.artcc)))
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
 * One notification for everything a single access save granted.
 *
 * `/me` lists *effective* permissions, role-derived ones included, so granting a role with twenty
 * permissions used to raise twenty-one notifications at once (VATUSA/OIS#348 review). A new role is
 * named and the permissions it brought go unsaid; otherwise a lone permission is named and several
 * are counted.
 */
export function summarizeGrants(fresh: Map<string, Entry>): Entry {
  const roles = [...fresh.keys()].filter((k) => k.startsWith("role:")).map((k) => k.slice(5));
  const permissions = [...fresh.keys()].filter((k) => k.startsWith("perm:")).map((k) => k.slice(5));
  const body =
    roles.length === 1
      ? `You were given the ${roles[0]} role.`
      : roles.length > 1
        ? `You were given the ${roles.join(", ")} roles.`
        : permissions.length === 1
          ? `You were given ${permissions[0]}.`
          : `You were given ${permissions.length} new permissions.`;
  return {title: "Access granted", body, route: "/profile"};
}

/**
 * Notifies when the signed-in user gains a permission or role.
 *
 * `access.granted` invalidates `["me"]`, so this just watches the refetched profile. Comparing
 * against the previous value is what makes it "granted" rather than "you have access" — the nudge
 * itself is broadcast to everyone and says nothing about who changed.
 */
export function AccessNotifier() {
  const {value: enabled} = useSetting<boolean>("notifications.access", false);
  const {data: me} = useMe();

  const held = React.useMemo(() => {
    const m = new Map<string, Entry>();
    // The entry text is unused — `summarizeGrants` writes the one notification — but the keys
    // are what the diff runs on. Permissions as well as roles: most grants are permissions.
    const entry = {title: "", body: "", route: "/profile"};
    for (const role of me?.role_names ?? []) m.set(`role:${role}`, entry);
    for (const permission of flattenPermissions(me?.permissions)) m.set(`perm:${permission}`, entry);
    return m;
  }, [me?.role_names, me?.permissions]);

  useNewKeys(held, !!me, "access", enabled, summarizeGrants);
  return null;
}

const REMINDER_TIERS = [
  {hours: 6, label: "6 hours"},
  {hours: 24, label: "24 hours"},
];

/** `Date.now()`, refreshed every minute. */
function useMinuteClock(): number {
  const [now, setNow] = React.useState(() => Date.now());
  React.useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(id);
  }, []);
  return now;
}

/**
 * Notifies 24h and 6h before an event the user has claimed an ACE position for.
 *
 * Mirrors the Discord reminder the scheduler sends. The backend nudges `events.reminder` on every
 * tick while any claim sits in a reminder window; this refetches the user's own claims and works
 * out which one is due. The window check is client-side because the nudge carries no payload.
 */
export function EventReminderNotifier() {
  const {value: enabled} = useSetting<boolean>("notifications.eventReminders", false);
  const {data: me} = useMe();

  const claims = useQuery({
    queryKey: ["my-ace-claims"],
    // Opened only for someone who asked for reminders and can hold a claim: the endpoint is gated
    // on `ace.requests.claim`, so for everyone else it was a 403 on every mount, swallowed, and a
    // request nobody opted into (VATUSA/OIS#348 review).
    enabled: !!me && can("notifications") && enabled && hasPermission(me, "ace.requests.claim"),
    queryFn: async () => {
      const {data} = await ois.GET("/api/v1/me/ace-claims");
      return data ?? [];
    },
  });

  // Whether a claim is due depends on the clock as much as on the data, and a refetch that returns
  // the same claims hands back the *same* `data` object — so keyed on `data` alone, crossing T-24h
  // or T-6h was never noticed and no reminder ever fired (VATUSA/OIS#348 review). Re-evaluate on
  // every refetch (the `events.reminder` nudge) and on a one-minute tick, whichever comes first —
  // a refetch's own timestamp is as good a "now" as the tick's.
  const now = Math.max(useMinuteClock(), claims.dataUpdatedAt);
  const due = React.useMemo(() => {
    const m = new Map<string, Entry>();
    for (const claim of claims.data ?? []) {
      const startsIn = new Date(claim.start_time).getTime() - now;
      if (startsIn <= 0) continue;
      const tier = REMINDER_TIERS.find((t) => startsIn <= t.hours * 3_600_000);
      if (!tier) continue;
      // Keyed by tier as well as claim, so the 24h and 6h reminders are two notifications, and
      // neither repeats.
      m.set(`${claim.claim_id}:${tier.hours}`, {
        title: `${claim.event_title} in ${tier.label}`,
        // `position` is optional — support can be requested without naming one.
        body: claim.position
          ? `You're claimed for ${claim.position}.`
          : `You're claimed for a position.`,
        route: `/planning/events/${claim.event_id}`,
      });
    }
    return m;
  }, [claims.data, now]);

  useNewKeys(due, claims.isSuccess, "eventReminders", enabled);
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
