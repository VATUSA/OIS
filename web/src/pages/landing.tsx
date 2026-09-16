import {Button} from "@ois/ui";
import {Link} from "@tanstack/react-router";
import {ArrowRight, CalendarClock, Gauge, type LucideIcon, Radar, TrendingUp, Waypoints, Wind} from "lucide-react";

import {Footer} from "@/components/footer";
import {login} from "@/lib/auth";

/** A tinted icon-chip accent — background + matching icon color, paired so they always read as one. */
type Accent = { bg: string; icon: string };

const ACCENT = {
  teal: { bg: "bg-series-1/15", icon: "text-series-1" },
  indigo: { bg: "bg-series-6/15", icon: "text-series-6" },
  amber: { bg: "bg-series-3/15", icon: "text-series-3" },
  rose: { bg: "bg-series-5/15", icon: "text-series-5" },
  emerald: { bg: "bg-series-2/15", icon: "text-series-2" },
  violet: { bg: "bg-series-4/15", icon: "text-series-4" },
} as const satisfies Record<string, Accent>;

function chipClass({ bg, icon }: Accent): string {
  return `flex size-9 shrink-0 items-center justify-center rounded-md ${bg} ${icon}`;
}

const FEATURES: { icon: LucideIcon; title: string; body: string; to: string; accent: Accent }[] = [
  {
    icon: Gauge,
    title: "Traffic management",
    body: "Metering programs, ground stops, and restrictions — issued, tracked, and shared live.",
    to: "/ops/tmu",
    accent: ACCENT.teal,
  },
  {
    icon: Waypoints,
    title: "Flow constrained areas",
    body: "Draw FCAs, sequence crossing traffic, and issue CFR releases against the live network.",
    to: "/ops/fca",
    accent: ACCENT.indigo,
  },
  {
    icon: CalendarClock,
    title: "Event planning",
    body: "Per-event airport rates, facility support, staffing, and TMI packages that go live on cue.",
    to: "/admin/planning/events",
    accent: ACCENT.amber,
  },
  {
    icon: Wind,
    title: "Runway balancer",
    body: "Assign arrivals to runways from live demand, with a rolling 10-minute board.",
    to: "/ops/runway",
    accent: ACCENT.rose,
  },
  {
    icon: Radar,
    title: "Facility maps",
    body: "A public, per-facility TMU map of live traffic with staff-editable color rules.",
    to: "/facility-map",
    accent: ACCENT.emerald,
  },
  {
    icon: TrendingUp,
    title: "Historical replay",
    body: "Scrub a past event or window and replay your dashboards at any instant.",
    to: "/admin/historical",
    accent: ACCENT.violet,
  },
];

/** A navigable tool/feature card: icon chip + title + one-liner + arrow. Shared between the
 * signed-out landing grid and (in principle) any future signed-in launchpad card grid, so the two
 * states read as one visual system. */
function FeatureCard({
  to,
  icon: Icon,
  accent,
  title,
  body,
}: {
  to: string;
  icon: LucideIcon;
  accent: Accent;
  title: string;
  body: string;
}) {
  return (
    <Link
      to={to as "/"}
      className="group flex items-start gap-3 rounded-lg border p-4 transition-colors hover:bg-accent/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
    >
      <span className={chipClass(accent)}>
        <Icon className="size-5" />
      </span>
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="font-semibold">{title}</span>
        <span className="text-sm text-muted-foreground">{body}</span>
      </div>
      <ArrowRight className="mt-1 size-4 shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
    </Link>
  );
}

/** The public landing at `/` for signed-out visitors (outside the app shell, with the site footer). */
export function LandingPage() {
  return (
    <div className="flex min-h-dvh flex-col bg-background text-foreground">
    <main className="mx-auto w-full max-w-7xl flex-1 px-4 py-8">
    <div className="flex flex-col gap-12 py-10">
      {/* Hero */}
      <div className="flex flex-col items-center gap-5 text-center">
        <Radar className="size-10 text-primary" />
        <h1 className="max-w-2xl text-4xl font-semibold tracking-tight sm:text-5xl">
          OIS
        </h1>
        <p className="max-w-xl text-muted-foreground">
          VATUSA's traffic-management and event-planning platform.
        </p>
        <Button size="lg" onClick={login}>
          Sign in with VATSIM
        </Button>
        <span className="text-xs text-muted-foreground">
          Or browse{" "}
          <Link to="/advisories" className="underline underline-offset-2 hover:text-foreground">
            advisories
          </Link>{" "}
          and{" "}
          <Link to="/facility-map" className="underline underline-offset-2 hover:text-foreground">
            facility maps
          </Link>{" "}
          without signing in.
        </span>
      </div>

      {/* Feature grid */}
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
        {FEATURES.map((f) => (
          <FeatureCard
            key={f.title}
            to={f.to}
            icon={f.icon}
            accent={f.accent}
            title={f.title}
            body={f.body}
          />
        ))}
      </div>
    </div>
    </main>
    <Footer />
    </div>
  );
}
