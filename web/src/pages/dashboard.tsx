import {Button, Card, CardContent, CardDescription, CardHeader, CardTitle,} from "@ois/ui";
import {CalendarClock, Gauge, LifeBuoy, type LucideIcon, ShieldCheck,} from "lucide-react";

import {login, useMe} from "@/lib/auth";

type Tool = {
  title: string;
  description: string;
  icon: LucideIcon;
  ready?: boolean;
};

const TOOLS: Tool[] = [
  {
    title: "Access control",
    description: "Fine-grained, per-ARTCC permissions with an audit trail.",
    icon: ShieldCheck,
    ready: true,
  },
  {
    title: "Events",
    description: "Pre / during / post event coordination and staffing.",
    icon: CalendarClock,
  },
  {
    title: "Traffic management",
    description: "NTML / ADV / TMI, flow programs, and delay reporting.",
    icon: Gauge,
  },
  {
    title: "ACE support",
    description: "Request and claim ACE coverage.",
    icon: LifeBuoy,
  },
];

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

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {TOOLS.map((tool) => (
          <Card key={tool.title} className="flex flex-col">
            <CardHeader>
              <div className="flex items-center gap-3">
                <span className="flex size-9 items-center justify-center rounded-md bg-primary/10 text-primary">
                  <tool.icon className="size-5" />
                </span>
                <CardTitle>{tool.title}</CardTitle>
              </div>
              <CardDescription>{tool.description}</CardDescription>
            </CardHeader>
            <CardContent className="mt-auto">
              <span className="text-xs font-medium text-muted-foreground">
                {tool.ready ? "Available" : "Coming soon"}
              </span>
            </CardContent>
          </Card>
        ))}
      </div>
    </div>
  );
}
