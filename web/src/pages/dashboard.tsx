import {Badge, Button, Card, CardContent} from "@ois/ui";
import {Link} from "@tanstack/react-router";
import {
  ArrowRight,
  CalendarClock,
  Gauge,
  type LucideIcon,
  Megaphone,
  OctagonX,
  Plane,
  Radar,
  Split,
  Timer,
  TrendingUp,
  Waypoints,
  Wind,
} from "lucide-react";

import {login, useMe} from "@/lib/auth";
import {useUpcomingEvents} from "@/lib/events";
import {useFeedStatus} from "@/lib/feed";
import {hasPermission} from "@/lib/permissions";
import {useGroundStops, usePrograms, useTmis} from "@/lib/tmu";
import {formatZuluFull, hhmmZulu} from "@/lib/time";

const FEATURES: { icon: LucideIcon; title: string; body: string }[] = [
  {
    icon: Gauge,
    title: "Traffic management",
    body: "Metering programs, ground stops, and restrictions — issued, tracked, and shared live.",
  },
  {
    icon: Waypoints,
    title: "Flow constrained areas",
    body: "Draw FCAs, sequence crossing traffic, and issue CFR releases against the live network.",
  },
  {
    icon: CalendarClock,
    title: "Event planning",
    body: "Per-event airport rates, facility support, staffing, and TMI packages that go live on cue.",
  },
  {
    icon: Wind,
    title: "Runway balancer",
    body: "Assign arrivals to runways from live demand, with a rolling 10-minute board.",
  },
  {
    icon: Radar,
    title: "Facility maps",
    body: "A public, per-facility TMU map of live traffic with staff-editable color rules.",
  },
  {
    icon: TrendingUp,
    title: "Historical replay",
    body: "Scrub a past event or window and replay your dashboards at any instant.",
  },
];

/** Public entry points that need no sign-in. */
function PublicLink({ to, icon: Icon, label }: { to: string; icon: LucideIcon; label: string }) {
  return (
    <Link
      to={to as "/"}
      className="flex items-center gap-1.5 rounded-md border px-3 py-1.5 text-sm font-medium transition-colors hover:bg-accent/40"
    >
      <Icon className="size-4 text-muted-foreground" />
      {label}
    </Link>
  );
}

function SignedOut() {
  return (
    <div className="flex flex-col gap-12 py-10">
      {/* Hero */}
      <div className="flex flex-col items-center gap-5 text-center">
        <Badge variant="secondary" className="uppercase tracking-wide">
          VATUSA operations
        </Badge>
        <h1 className="max-w-2xl text-4xl font-semibold tracking-tight sm:text-5xl">
          Event Operational Information System
        </h1>
        <p className="max-w-xl text-muted-foreground">
          The traffic-management and event-planning platform for VATUSA — flow control, FCAs, runway
          balancing, and live facility maps, all in one place.
        </p>
        <div className="flex flex-col items-center gap-3">
          <Button size="lg" onClick={login}>
            Sign in with VATSIM
          </Button>
          <div className="flex flex-wrap items-center justify-center gap-2 pt-2">
            <span className="text-xs text-muted-foreground">Or explore without signing in:</span>
            <PublicLink to="/advisories" icon={Megaphone} label="Advisories" />
            <PublicLink to="/advisories/fcas" icon={Waypoints} label="FCA overview" />
            <PublicLink to="/facility-map" icon={Radar} label="Facility maps" />
          </div>
        </div>
      </div>

      {/* Feature grid */}
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {FEATURES.map((f) => (
          <Card key={f.title}>
            <CardContent className="flex flex-col gap-2 pt-6">
              <span className="flex size-9 items-center justify-center rounded-md bg-primary/10 text-primary">
                <f.icon className="size-5" />
              </span>
              <span className="font-semibold">{f.title}</span>
              <span className="text-sm text-muted-foreground">{f.body}</span>
            </CardContent>
          </Card>
        ))}
      </div>
    </div>
  );
}

function StatTile({
  icon: Icon,
  label,
  value,
  tone,
}: {
  icon: LucideIcon;
  label: string;
  value: string | number;
  tone?: string;
}) {
  return (
    <Card>
      <CardContent className="flex items-center gap-3 pt-6">
        <span
          className={
            "flex size-9 items-center justify-center rounded-md bg-primary/10 " +
            (tone ?? "text-primary")
          }
        >
          <Icon className="size-5" />
        </span>
        <div className="flex flex-col">
          <span className="text-2xl font-semibold tabular-nums leading-none">
            {value}
          </span>
          <span className="text-xs uppercase tracking-wide text-muted-foreground">
            {label}
          </span>
        </div>
      </CardContent>
    </Card>
  );
}

/** A card that frames one operational list, with a header and a "view all" link. */
function Section({
  title,
  count,
  to,
  search,
  children,
}: {
  title: string;
  count: number;
  to: string;
  search?: Record<string, string>;
  children: React.ReactNode;
}) {
  return (
    <Card className="flex flex-col">
      <CardContent className="flex flex-1 flex-col gap-3 pt-6">
        <div className="flex items-center justify-between">
          <span className="text-sm font-semibold">
            {title}
            <span className="ml-2 text-muted-foreground">{count}</span>
          </span>
          <Link
            to={to}
            search={search}
            className="flex items-center gap-1 text-xs text-muted-foreground transition-colors hover:text-foreground"
          >
            View all <ArrowRight className="size-3.5" />
          </Link>
        </div>
        {children}
      </CardContent>
    </Card>
  );
}

function spacing(trail: number, mit: number): string {
  if (mit > 0) return `${mit} MIT`;
  if (trail > 0) return `${trail} MINIT`;
  return "no spacing";
}

function Empty({ children }: { children: React.ReactNode }) {
  return <p className="py-6 text-center text-sm text-muted-foreground">{children}</p>;
}

/** A tool tile in the signed-in launchpad. */
/** A coarse "in Xd Yh" / "Xh Ym" / "Ym" from a millisecond delta. */
function countdown(ms: number): string {
  const abs = Math.abs(ms);
  const d = Math.floor(abs / 86_400_000);
  const h = Math.floor((abs % 86_400_000) / 3_600_000);
  const m = Math.floor((abs % 3_600_000) / 60_000);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

/**
 * The event happening right now, or — if none — the next one scheduled. The homepage's headline: a
 * controller lands and immediately sees whether they're mid-event and how long is left, or what's next.
 * Gated on events access by the caller.
 */
function FeaturedEvent() {
  const events = useUpcomingEvents();
  const now = Date.now();
  const sorted = (events.data ?? [])
    .slice()
    .sort((a, b) => new Date(a.start_time).getTime() - new Date(b.start_time).getTime());
  const current = sorted.find(
    (e) => new Date(e.start_time).getTime() <= now && now <= new Date(e.end_time).getTime(),
  );
  const next = sorted.find((e) => new Date(e.start_time).getTime() > now);
  const featured = current ?? next;
  const alsoUpcoming = sorted.filter((e) => new Date(e.start_time).getTime() > now && e !== featured).slice(0, 3);

  if (!events.data) {
    return (
      <Card>
        <CardContent className="py-10 text-center text-sm text-muted-foreground">Loading events…</CardContent>
      </Card>
    );
  }
  if (!featured) {
    return (
      <Card>
        <CardContent className="flex items-center gap-3 py-8 text-sm text-muted-foreground">
          <CalendarClock className="size-5" />
          No events on the calendar right now.
        </CardContent>
      </Card>
    );
  }

  const live = !!current;
  const start = new Date(featured.start_time).getTime();
  const end = new Date(featured.end_time).getTime();

  return (
    <Card className="overflow-hidden">
      <div className="flex flex-col gap-5 p-6 sm:flex-row">
        {featured.banner_image_url && (
          <img
            src={featured.banner_image_url}
            alt=""
            className="h-32 w-full rounded-md object-cover sm:h-auto sm:w-56"
          />
        )}
        <div className="flex min-w-0 flex-1 flex-col gap-3">
          <div className="flex flex-wrap items-center gap-2">
            {live ? (
              <span className="flex items-center gap-1.5 rounded-full bg-emerald-500/15 px-2 py-0.5 text-xs font-semibold uppercase tracking-wide text-emerald-500">
                <span className="size-1.5 animate-pulse rounded-full bg-emerald-500" />
                Live now
              </span>
            ) : (
              <Badge variant="secondary" className="uppercase tracking-wide">
                Next event
              </Badge>
            )}
            <span className="text-sm font-medium text-muted-foreground">
              {live ? `ends in ${countdown(end - now)}` : `starts in ${countdown(start - now)}`}
            </span>
          </div>

          <Link
            to="/planning/events/$eventId"
            params={{ eventId: String(featured.id) }}
            className="text-2xl font-semibold tracking-tight hover:text-primary"
          >
            {featured.title}
          </Link>

          <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-sm text-muted-foreground">
            {featured.facility && (
              <span className="font-mono font-medium text-foreground">{featured.facility}</span>
            )}
            <span className="flex items-center gap-1.5">
              <CalendarClock className="size-3.5" />
              {formatZuluFull(featured.start_time)} – {formatZuluFull(featured.end_time)}
            </span>
          </div>

          {alsoUpcoming.length > 0 && (
            <div className="mt-1 flex flex-col gap-1 border-t pt-3 text-sm">
              <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                Also coming up
              </span>
              {alsoUpcoming.map((e) => (
                <Link
                  key={e.id}
                  to="/planning/events/$eventId"
                  params={{ eventId: String(e.id) }}
                  className="flex items-center justify-between gap-2 hover:text-primary"
                >
                  <span className="min-w-0 truncate">{e.title}</span>
                  <span className="flex shrink-0 items-center gap-2 text-muted-foreground">
                    {e.facility && <span className="font-mono text-xs">{e.facility}</span>}
                    <span className="text-xs">{formatZuluFull(e.start_time)}</span>
                  </span>
                </Link>
              ))}
            </div>
          )}
        </div>
      </div>
    </Card>
  );
}

function Overview() {
  const { data: me } = useMe();
  const { data: feed } = useFeedStatus();

  const canPrograms = hasPermission(me, "tmu.program.read");
  const canGroundStops = hasPermission(me, "tmu.groundstop.read");
  const canTmis = hasPermission(me, "tmu.tmi.read");
  const canPlan = hasPermission(me, "events.plan.read");

  const programs = usePrograms();
  const groundStops = useGroundStops();
  const tmis = useTmis();

  const progList = canPrograms ? (programs.data ?? []) : [];
  const gsList = (canGroundStops ? (groundStops.data ?? []) : []).filter(
    (g) => g.status !== "cancelled" && g.status !== "expired",
  );
  const tmiList = (canTmis ? (tmis.data ?? []) : []).filter(
    (t) => t.status === "published",
  );

  return (
    <div className="flex flex-col gap-6">
      {/* Headline: the current or next event */}
      {canPlan && <FeaturedEvent />}

      {/* Live snapshot tiles */}
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        <StatTile
          icon={Plane}
          label="Pilots online"
          value={feed ? feed.pilots : "—"}
          tone={feed?.healthy ? "text-emerald-500" : "text-muted-foreground"}
        />
        {canPrograms && (
          <StatTile icon={Gauge} label="Metering programs" value={progList.length} />
        )}
        {canGroundStops && (
          <StatTile
            icon={OctagonX}
            label="Ground stops"
            value={gsList.length}
            tone={gsList.length > 0 ? "text-destructive" : "text-primary"}
          />
        )}
        {canTmis && (
          <StatTile icon={Split} label="Restrictions" value={tmiList.length} />
        )}
      </div>

      {/* Operational lists */}
      <div className="grid gap-4 lg:grid-cols-3">
        {canPrograms && (
          <Section title="Metering programs" count={progList.length} to="/ops/tmu">
            {progList.length === 0 ? (
              <Empty>No active programs.</Empty>
            ) : (
              <ul className="flex flex-col divide-y divide-border/60">
                {progList.slice(0, 6).map((p) => (
                  <li
                    key={p.icao}
                    className="flex items-center justify-between gap-2 py-2 text-sm"
                  >
                    <span className="font-mono font-semibold">{p.icao}</span>
                    <span className="flex items-center gap-2 text-muted-foreground">
                      <Badge variant="secondary">AAR {p.aar}</Badge>
                      <span className="text-xs">{spacing(p.trail, p.mit)}</span>
                      {p.active_until && (
                        <span className="text-xs">· {hhmmZulu(p.active_until)}</span>
                      )}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </Section>
        )}

        {canGroundStops && (
          <Section
            title="Ground stops"
            count={gsList.length}
            to="/ops/tmu"
            search={{ tab: "ground-stops" }}
          >
            {gsList.length === 0 ? (
              <Empty>No ground stops.</Empty>
            ) : (
              <ul className="flex flex-col divide-y divide-border/60">
                {gsList.slice(0, 6).map((gs) => (
                  <li
                    key={gs.id}
                    className="flex items-center justify-between gap-2 py-2 text-sm"
                  >
                    <span className="font-mono font-semibold">{gs.airport}</span>
                    <span className="flex items-center gap-2 text-muted-foreground">
                      <span className="text-xs">
                        {gs.scope ? gs.scope : "all departures"}
                      </span>
                      <Badge variant="outline">
                        {gs.until ? `${gs.until}z` : "UFN"}
                      </Badge>
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </Section>
        )}

        {canTmis && (
          <Section
            title="Restrictions"
            count={tmiList.length}
            to="/ops/tmu"
            search={{ tab: "restrictions" }}
          >
            {tmiList.length === 0 ? (
              <Empty>No published restrictions.</Empty>
            ) : (
              <ul className="flex flex-col divide-y divide-border/60">
                {tmiList.slice(0, 6).map((t) => (
                  <li key={t.id} className="flex flex-col gap-0.5 py-2 text-sm">
                    <span className="flex items-center gap-1.5 font-mono text-xs text-muted-foreground">
                      {t.requesting}
                      <ArrowRight className="size-3" />
                      {t.providing}
                    </span>
                    <span className="truncate">{t.restriction}</span>
                  </li>
                ))}
              </ul>
            )}
          </Section>
        )}
      </div>

      {!canPrograms && !canGroundStops && !canTmis && !canPlan && (
        <Card>
          <CardContent className="flex items-center gap-3 py-8 text-sm text-muted-foreground">
            <Timer className="size-5" />
            You don&apos;t have traffic-management access yet. Ask an ARTCC admin
            to grant TMU permissions.
          </CardContent>
        </Card>
      )}
    </div>
  );
}

export function DashboardPage() {
  const { data: me, isLoading } = useMe();

  if (isLoading) {
    return <div className="py-24 text-center text-muted-foreground">Loading…</div>;
  }
  if (!me) return <SignedOut />;

  return (
    <div className="flex flex-col gap-8">
      <div className="flex flex-col gap-1">
        <h1 className="text-2xl font-semibold tracking-tight">
          Welcome, {me.display_name}
        </h1>
        <p className="text-muted-foreground">
          {me.server_admin ? "Server admin" : me.role_names.join(", ") || "Controller"}
          {me.rating ? ` · ${me.rating}` : ""}
        </p>
      </div>

      <Overview />
    </div>
  );
}
