import {Badge, Button, Card, CardContent} from "@ois/ui";
import {Link} from "@tanstack/react-router";
import {ArrowRight, Gauge, type LucideIcon, OctagonX, Plane, Split, Timer,} from "lucide-react";

import {login, useMe} from "@/lib/auth";
import {useFeedStatus} from "@/lib/feed";
import {hasPermission} from "@/lib/permissions";
import {useGroundStops, usePrograms, useTmis} from "@/lib/tmu";
import {hhmmZulu} from "@/lib/time";

function SignedOut() {
  return (
    <div className="flex flex-col items-center justify-center gap-6 py-24 text-center">
      <div className="flex flex-col items-center gap-2">
        <h1 className="text-3xl font-semibold tracking-tight">
          Event Operational Information System
        </h1>
        <p className="max-w-md text-muted-foreground">
          The VATUSA operations platform. Sign in with your VATSIM account to
          continue.
        </p>
      </div>
      <Button size="lg" onClick={login}>
        Sign in with VATSIM
      </Button>
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
  children,
}: {
  title: string;
  count: number;
  to: string;
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
  if (trail > 0) return `${trail} MIN`;
  return "no spacing";
}

function Empty({ children }: { children: React.ReactNode }) {
  return <p className="py-6 text-center text-sm text-muted-foreground">{children}</p>;
}

function Overview() {
  const { data: me } = useMe();
  const { data: feed } = useFeedStatus();

  const canPrograms = hasPermission(me, "tmu.program.read");
  const canGroundStops = hasPermission(me, "tmu.groundstop.read");
  const canTmis = hasPermission(me, "tmu.tmi.read");

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
          <Section title="Ground stops" count={gsList.length} to="/ops/tmu">
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
          <Section title="Restrictions" count={tmiList.length} to="/ops/tmu">
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

      {!canPrograms && !canGroundStops && !canTmis && (
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
