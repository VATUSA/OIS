import {Button, buttonVariants, Card, EmptyState, MetricCard, QueryState, StatusPill} from "@ois/ui";
import {Link, type LinkProps} from "@tanstack/react-router";
import {
  ArrowRight,
  ArrowUpRight,
  BookOpen,
  CalendarClock,
  Gauge,
  type LucideIcon,
  Map as MapIcon,
  Megaphone,
  OctagonX,
  Plane,
  PlaneTakeoff,
  Radar,
  Split,
  Timer,
  TrendingUp,
  Waypoints,
  Wind,
} from "lucide-react";

import vatusaLogo from "@/assets/vatusa-logo.png";
import {DOCS_URL} from "@/lib/api";
import {login, useMe} from "@/lib/auth";
import {useUpcomingEvents} from "@/lib/events";
import {useFeedStatus} from "@/lib/feed";
import {hasPermission} from "@/lib/permissions";
import {useGroundStops, usePrograms, useTmis} from "@/lib/tmu";
import {formatZuluFull, hhmmZulu} from "@/lib/time";

/** A card that frames one operational summary list, with a header and a "view all" link. */
function Section({
  title,
  count,
  link,
  children,
}: {
  title: string;
  count: number;
  link: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <Card className="flex flex-col gap-2 p-5">
      <div className="flex items-center justify-between gap-2">
        <h2 className="flex items-center gap-2 text-xl font-bold text-ink">
          {title}
          <span className="rounded-full bg-chip px-2 py-0.5 font-mono text-xs font-semibold text-ink-2">{count}</span>
        </h2>
        {link}
      </div>
      {children}
    </Card>
  );
}

const viewAllClass = buttonVariants({ variant: "ghost", size: "sm" });
const rowClass = "flex items-center justify-between gap-2 border-b border-line-soft py-2 text-sm last:border-b-0";

function spacing(trail: number, mit: number): string {
  if (mit > 0) return `${mit} MIT`;
  if (trail > 0) return `${trail} MINIT`;
  return "no spacing";
}

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
        <QueryState isLoading loading="Loading events…" />
      </Card>
    );
  }
  if (!featured) {
    return (
      <Card>
        <EmptyState icon={CalendarClock}>No events on the calendar right now.</EmptyState>
      </Card>
    );
  }

  const live = !!current;
  const start = new Date(featured.start_time).getTime();
  const end = new Date(featured.end_time).getTime();

  return (
    <Card className="overflow-hidden">
      <div className="flex flex-col gap-5 p-5 sm:flex-row">
        {featured.banner_image_url && (
          <img
            src={featured.banner_image_url}
            alt=""
            className="h-32 w-full rounded-sm object-cover sm:h-auto sm:w-56"
          />
        )}
        <div className="flex min-w-0 flex-1 flex-col gap-3">
          <div className="flex flex-wrap items-center gap-2">
            {live ? (
              <StatusPill tone="good" dot>
                Live now
              </StatusPill>
            ) : (
              <StatusPill tone="brand">Next event</StatusPill>
            )}
            <span className="font-mono text-xs text-ink-2">
              {live ? `ends in ${countdown(end - now)}` : `starts in ${countdown(start - now)}`}
            </span>
          </div>

          <Link
            to="/admin/planning/events/$eventId"
            params={{ eventId: String(featured.id) }}
            className="text-2xl font-bold tracking-tight text-ink transition-colors hover:text-brand-ink"
          >
            {featured.title}
          </Link>

          <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-sm text-ink-2">
            {featured.facility && <span className="font-mono font-semibold text-ink">{featured.facility}</span>}
            <span className="flex items-center gap-1.5 font-mono text-[13px]">
              <CalendarClock className="size-3.5 text-ink-3" />
              {formatZuluFull(featured.start_time)} – {formatZuluFull(featured.end_time)}
            </span>
          </div>

          {alsoUpcoming.length > 0 && (
            <div className="mt-1 flex flex-col gap-1 border-t border-line-soft pt-3 text-sm">
              <span className="text-xs font-semibold text-ink-3">Also coming up</span>
              {alsoUpcoming.map((e) => (
                <Link
                  key={e.id}
                  to="/admin/planning/events/$eventId"
                  params={{ eventId: String(e.id) }}
                  className="flex items-center justify-between gap-2 py-0.5 text-ink transition-colors hover:text-brand-ink"
                >
                  <span className="min-w-0 truncate">{e.title}</span>
                  <span className="flex shrink-0 items-center gap-3 font-mono text-xs text-ink-2">
                    {e.facility && <span>{e.facility}</span>}
                    <span>{formatZuluFull(e.start_time)}</span>
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
  const tmiList = (canTmis ? (tmis.data ?? []) : []).filter((t) => t.status === "published");

  return (
    <div className="flex flex-col gap-6">
      {/* Headline: the current or next event */}
      {canPlan && <FeaturedEvent />}

      {/* Live snapshot */}
      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        <MetricCard
          icon={Plane}
          label="Pilots online"
          value={feed ? feed.pilots : "—"}
          sub={
            feed && (
              <StatusPill tone={feed.healthy ? "good" : "bad"} dot>
                {feed.healthy ? "Feed healthy" : "Feed unhealthy"}
              </StatusPill>
            )
          }
        />
        {canPrograms && <MetricCard icon={Gauge} label="Metering programs" value={progList.length} />}
        {canGroundStops && (
          <MetricCard
            icon={OctagonX}
            label="Ground stops"
            value={gsList.length}
            tone={gsList.length > 0 ? "bad" : undefined}
          />
        )}
        {canTmis && <MetricCard icon={Split} label="Restrictions" value={tmiList.length} />}
      </div>

      {/* Operational summaries */}
      {(canPrograms || canGroundStops || canTmis) && (
        <div className="grid gap-3 lg:grid-cols-3">
          {canPrograms && (
            <Section
              title="Metering programs"
              count={progList.length}
              link={
                <Link to="/ops/tmu" className={viewAllClass}>
                  View all <ArrowRight />
                </Link>
              }
            >
              {progList.length === 0 ? (
                <EmptyState icon={Gauge}>No active programs.</EmptyState>
              ) : (
                <ul className="flex flex-col">
                  {progList.slice(0, 6).map((p) => (
                    <li key={p.icao} className={rowClass}>
                      <span className="font-mono font-semibold text-ink">{p.icao}</span>
                      <span className="flex items-center gap-2 font-mono text-xs text-ink-2">
                        <StatusPill>AAR {p.aar}</StatusPill>
                        <span>{spacing(p.trail, p.mit)}</span>
                        {p.active_until && <span>· {hhmmZulu(p.active_until)}</span>}
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
              link={
                <Link to="/ops/tmu" search={{ tab: "ground-stops" }} className={viewAllClass}>
                  View all <ArrowRight />
                </Link>
              }
            >
              {gsList.length === 0 ? (
                <EmptyState icon={OctagonX}>No ground stops.</EmptyState>
              ) : (
                <ul className="flex flex-col">
                  {gsList.slice(0, 6).map((gs) => (
                    <li key={gs.id} className={rowClass}>
                      <span className="font-mono font-semibold text-ink">{gs.airport}</span>
                      <span className="flex min-w-0 items-center gap-2 text-ink-2">
                        <span className="truncate text-xs">{gs.scope ? gs.scope : "all departures"}</span>
                        <StatusPill className="font-mono">{gs.until ? `${gs.until}z` : "UFN"}</StatusPill>
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
              link={
                <Link to="/ops/tmu" search={{ tab: "restrictions" }} className={viewAllClass}>
                  View all <ArrowRight />
                </Link>
              }
            >
              {tmiList.length === 0 ? (
                <EmptyState icon={Split}>No published restrictions.</EmptyState>
              ) : (
                <ul className="flex flex-col">
                  {tmiList.slice(0, 6).map((t) => (
                    <li key={t.id} className="flex flex-col gap-0.5 border-b border-line-soft py-2 text-sm last:border-b-0">
                      <span className="flex items-center gap-1.5 font-mono text-xs text-ink-2">
                        {t.requesting}
                        <ArrowRight className="size-3 text-ink-3" />
                        {t.providing}
                      </span>
                      <span className="truncate text-ink">{t.restriction}</span>
                    </li>
                  ))}
                </ul>
              )}
            </Section>
          )}
        </div>
      )}

      {!canPrograms && !canGroundStops && !canTmis && !canPlan && (
        <Card>
          <EmptyState icon={Timer} title="No traffic-management access yet">
            Ask an ARTCC admin to grant TMU permissions.
          </EmptyState>
        </Card>
      )}
    </div>
  );
}

export function DashboardPage() {
  const { data: me, isLoading } = useMe();

  if (isLoading) return <QueryState isLoading className="py-24" />;
  // Signed out, "/" renders the public landing outside the shell (see RootLayout).
  if (!me) return null;

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col gap-1">
        <h1 className="text-[28px] font-bold leading-tight tracking-[-0.02em] text-ink">
          Welcome, {me.display_name}
        </h1>
        <p className="text-[15px] text-ink-2">
          {me.server_admin ? "Server admin" : me.role_names.join(", ") || "Controller"}
          {me.rating ? ` · ${me.rating}` : ""}
        </p>
      </div>

      <Overview />
    </div>
  );
}
