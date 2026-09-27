import {useEffect, useMemo, useRef, useState} from "react";
import {cn, toneBg, toneText, type Tone} from "@ois/ui";
import {AlertOctagon, X} from "lucide-react";

import {useMe} from "@/lib/auth";
import {hasPermission} from "@/lib/permissions";
import {inRestrictionScope, restrictionFacilities} from "@/lib/restriction-scope";
import {useHistoricalAt} from "@/lib/historical-context";
import {useGroundStops, usePrograms, useTmis, type GroundStop, type Program, type Tmi} from "@/lib/tmu";
import {useGdps, type Gdp} from "@/lib/gdp";

/** How long a restriction popup stays up before it auto-closes. */
const ALERT_MS = 15_000;

interface RestrictionAlert {
  key: string;
  kind: string;
  tone: Tone;
  title: string;
  lines: string[];
}

const TONES = {
  groundStop: "bad",
  gdp: "warn",
  program: "brand",
  tmi: "neutral",
} as const satisfies Record<string, Tone>;

function groundStopAlert(g: GroundStop): RestrictionAlert {
  return {
    key: `gs:${g.id}`,
    kind: "Ground Stop",
    tone: TONES.groundStop,
    title: g.airport,
    lines: [
      g.scope.trim() ? `Scope: ${g.scope.trim()}` : "Field-wide (all departures)",
      g.until ? `Until ${g.until}z` : "Until further notice",
    ],
  };
}

function gdpAlert(g: Gdp): RestrictionAlert {
  return {
    key: `gdp:${g.id}`,
    kind: "Ground Delay Program",
    tone: TONES.gdp,
    title: g.airport,
    lines: [
      `AAR ${g.aar}`,
      `${g.start_time}–${g.end_time}z`,
      ...(g.scope.trim() ? [`Scope: ${g.scope.trim()}`] : []),
    ],
  };
}

function tmiAlert(t: Tmi): RestrictionAlert {
  return {
    key: `tmi:${t.id}`,
    kind: "Restriction",
    tone: TONES.tmi,
    title: `${t.requesting} → ${t.providing}`,
    lines: [t.decoded || t.restriction],
  };
}

function programAlert(p: Program): RestrictionAlert {
  const bits = [
    p.mit > 0 ? `${p.mit} MIT` : null,
    p.aar > 0 ? `AAR ${p.aar}` : null,
    p.jets_only ? "jets only" : null,
  ].filter(Boolean) as string[];
  return {
    key: `prog:${p.icao}`,
    kind: "Metering Program",
    tone: TONES.program,
    title: p.icao,
    lines: bits.length ? [bits.join(" · ")] : ["Metering active"],
  };
}

/**
 * Broadcast popups: when a new restriction (ground stop, GDP, TMI, or metering program) is initiated,
 * a controller sees a prominent alert that auto-closes after {@link ALERT_MS}. Realtime nudges the
 * underlying lists, so it surfaces near-instantly. Only *new* restrictions fire — the set active when a
 * page first loads is captured silently. Mounted once in the root layout; gated to controllers below.
 *
 * Scoped to the user's own ARTCCs unless they read TMU nationally (VATUSA/OIS#405) — a ZDC controller
 * was previously alerted to every restriction in the country. This component is also the single owner
 * of "which restrictions are genuinely new", so the desktop notifier built on it (#348) inherits the
 * same audience rather than deciding it a second time.
 */
export function RestrictionAlerts() {
  const { data: me } = useMe();
  // Same audience as the feed watcher: controllers who actually run traffic management.
  if (!hasPermission(me, "tmu.program.read")) return null;
  return <RestrictionAlertsInner />;
}

function RestrictionAlertsInner() {
  const live = useHistoricalAt() == null;
  const { data: me } = useMe();
  const groundStops = useGroundStops();
  const gdps = useGdps();
  const tmis = useTmis();
  const programs = usePrograms();

  // Memoized because a fresh Set every render would make `active`'s memo below recompute every render.
  const facilities = useMemo(() => restrictionFacilities(me), [me]);

  // Identity keys currently "active" (published / present) and in scope, independent of live mode.
  const active = useMemo(() => {
    const mine = (...artccs: (string | null | undefined)[]) =>
      inRestrictionScope(facilities, artccs);
    const m = new Map<string, RestrictionAlert>();
    for (const g of groundStops.data ?? [])
      if (g.status === "published" && mine(g.artcc)) m.set(`gs:${g.id}`, groundStopAlert(g));
    for (const g of gdps.data ?? [])
      if (g.status === "published" && mine(g.artcc)) m.set(`gdp:${g.id}`, gdpAlert(g));
    for (const t of tmis.data ?? [])
      if (t.status === "published" && mine(t.requesting_artcc, t.providing_artcc))
        m.set(`tmi:${t.id}`, tmiAlert(t));
    for (const p of programs.data ?? [])
      if (mine(p.artcc)) m.set(`prog:${p.icao}`, programAlert(p));
    return m;
  }, [groundStops.data, gdps.data, tmis.data, programs.data, facilities]);

  // Keys we've already seen. Null until the first full load, so pre-existing restrictions never fire.
  const known = useRef<Set<string> | null>(null);
  const [alerts, setAlerts] = useState<RestrictionAlert[]>([]);

  const settled =
    !groundStops.isPending && !gdps.isPending && !tmis.isPending && !programs.isPending;

  // The scope `known` was last seeded for. When it changes — the VATUSA profile syncs after sign-in,
  // a visit is added, national TMU is granted — every restriction already running in the newly
  // covered scope enters `active` at once. Diffed, each one announced itself as just initiated, and
  // someone made national was alerted to the whole country (VATUSA/OIS#405 review). A new scope is
  // seeded silently instead, the same as the first load.
  const seededFor = useRef(facilities);

  useEffect(() => {
    // Historical replay swaps the lists to past data — don't alert, and don't disturb `known` so the
    // live set is intact when we return.
    if (!live) return;
    if (known.current == null || seededFor.current !== facilities) {
      if (settled) {
        known.current = new Set(active.keys());
        seededFor.current = facilities;
      }
      return;
    }
    const fresh: RestrictionAlert[] = [];
    for (const [key, a] of active) {
      if (!known.current.has(key)) {
        known.current.add(key);
        fresh.push(a);
      }
    }
    // Forget keys that dropped off so a cancel-then-reissue alerts again.
    for (const key of [...known.current]) if (!active.has(key)) known.current.delete(key);
    if (fresh.length) setAlerts((prev) => [...prev, ...fresh]);
  }, [live, settled, active, facilities]);

  const dismiss = (key: string) => setAlerts((prev) => prev.filter((a) => a.key !== key));

  if (!alerts.length) return null;
  return (
    <div className="pointer-events-none fixed inset-x-0 top-3 z-[900] flex flex-col items-center gap-2 px-3">
      {alerts.map((a) => (
        <AlertCard key={a.key} alert={a} onClose={() => dismiss(a.key)} />
      ))}
    </div>
  );
}

function AlertCard({ alert, onClose }: { alert: RestrictionAlert; onClose: () => void }) {
  const [width, setWidth] = useState("100%");
  useEffect(() => {
    // Kick the shrink-to-zero transition on the next frame, and auto-close when the window is up.
    const raf = requestAnimationFrame(() => setWidth("0%"));
    const timer = window.setTimeout(onClose, ALERT_MS);
    return () => {
      cancelAnimationFrame(raf);
      window.clearTimeout(timer);
    };
  }, [onClose]);

  return (
    <div
      className="pointer-events-auto w-full max-w-sm overflow-hidden rounded-md border border-line bg-panel"
      role="alert"
    >
      <div className="flex items-start gap-3 p-3">
        <AlertOctagon className={cn("mt-0.5 size-5 shrink-0", toneText[alert.tone])} />
        <div className="min-w-0 flex-1">
          <div className="flex items-center justify-between gap-2">
            <span className={cn("text-xs font-semibold", toneText[alert.tone])}>{alert.kind} initiated</span>
            <button
              type="button"
              aria-label="Dismiss"
              onClick={onClose}
              className="rounded-xs text-ink-3 transition-colors hover:text-ink"
            >
              <X className="size-4" />
            </button>
          </div>
          <div className="mt-0.5 font-mono font-semibold text-ink">{alert.title}</div>
          {alert.lines.map((l, i) => (
            <div key={i} className="text-sm text-ink-2">
              {l}
            </div>
          ))}
        </div>
      </div>
      <div className="h-0.5 w-full bg-line-soft">
        <div
          className={cn("h-full", toneBg[alert.tone])}
          style={{ width, transition: `width ${ALERT_MS}ms linear` }}
        />
      </div>
    </div>
  );
}
