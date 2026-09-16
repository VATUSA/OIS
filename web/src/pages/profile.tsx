import {useMemo} from "react";

import {Avatar, AvatarFallback, Card, EmptyState, QueryState, StatusPill} from "@ois/ui";
import {Building2, CalendarDays, KeyRound, LogIn, MapPin, MessageSquare, RefreshCw, ShieldCheck} from "lucide-react";
import type {LucideIcon} from "lucide-react";

import {usePageHeader} from "@/components/shell/page-meta";
import {useMe} from "@/lib/auth";
import {useFacilities} from "@/lib/admin";
import {useDiscordLink} from "@/lib/integration";

function initials(name: string): string {
  return name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((p) => p[0]?.toUpperCase() ?? "")
    .join("");
}

function fmtDate(iso: string | null | undefined): string {
  if (!iso) return "—";
  const d = new Date(iso);
  return Number.isNaN(d.getTime())
    ? "—"
    : d.toLocaleDateString(undefined, {
        year: "numeric",
        month: "short",
        day: "numeric",
      });
}

/** A profile section: a 20/700 title with an optional icon and right-aligned note. */
function Section({
  title,
  icon: Icon,
  note,
  children,
}: {
  title: string;
  icon?: LucideIcon;
  note?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <Card className="flex flex-col gap-4 p-5">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 className="flex items-center gap-2 text-xl font-bold text-ink">
          {Icon && <Icon className="size-4 text-ink-3" />}
          {title}
        </h2>
        {note && <span className="text-xs text-ink-3">{note}</span>}
      </div>
      {children}
    </Card>
  );
}

/** Discord link status — read-only, synced from the member's VATUSA profile. */
function DiscordCard() {
  const { data: link } = useDiscordLink();

  return (
    <Section title="Discord" icon={MessageSquare}>
      <p className="text-sm text-ink-2">
        Synced from your VATUSA profile — it lets bot actions (like claiming ACE requests) be attributed
        to you.
      </p>
      {link?.linked ? (
        <div className="flex items-center gap-2 text-sm">
          <StatusPill tone="good" dot>
            Connected
          </StatusPill>
          <span className="text-ink-2">Linked via VATUSA.</span>
        </div>
      ) : (
        <p className="text-sm text-ink-3">
          No Discord on file. Add your Discord account to your VATUSA profile, then sign in to OIS to sync
          it.
        </p>
      )}
    </Section>
  );
}

function Field({ label, value, mono }: { label: string; value: React.ReactNode; mono?: boolean }) {
  return (
    <div className="flex flex-col gap-1">
      <span className="text-xs font-semibold text-ink-3">{label}</span>
      <span className={mono ? "font-mono text-[13px] font-semibold text-ink" : "text-sm font-semibold text-ink"}>
        {value}
      </span>
    </div>
  );
}

export function ProfilePage() {
  const { data: me, isLoading } = useMe();
  const { data: facilities } = useFacilities();

  const facName = useMemo(() => {
    const map = new Map<string, string>();
    for (const f of facilities ?? []) map.set(f.id, f.name);
    return (id: string) => map.get(id) ?? null;
  }, [facilities]);

  const v = me?.vatusa ?? null;
  // Group VATUSA roles by facility for a tidy list.
  const rolesByFacility = useMemo(() => {
    const map = new Map<string, string[]>();
    for (const r of v?.roles ?? []) {
      const list = map.get(r.facility) ?? [];
      list.push(r.role);
      map.set(r.facility, list);
    }
    return [...map.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  }, [v]);

  usePageHeader({ subtitle: "Your VATSIM identity, VATUSA membership, and OIS access." });

  if (isLoading) return <QueryState isLoading />;
  if (!me) return <EmptyState icon={LogIn}>Sign in to view your profile.</EmptyState>;

  const oisRoles = me.role_names.filter((r) => r !== "USER");
  const homeFacName = v?.home_facility ? facName(v.home_facility) : null;

  return (
    <div className="flex w-full max-w-3xl flex-col gap-4">
      <Card className="flex items-center gap-4 p-5">
        <Avatar className="size-14 text-lg">
          <AvatarFallback>{initials(me.display_name)}</AvatarFallback>
        </Avatar>
        <div className="flex min-w-0 flex-col gap-1">
          <span className="truncate text-xl font-bold text-ink">{me.display_name}</span>
          <span className="flex flex-wrap items-center gap-x-3 gap-y-1 text-sm text-ink-2">
            <span className="font-mono text-[13px]">CID {me.cid}</span>
            {me.rating && <StatusPill>{me.rating}</StatusPill>}
            {me.email && <span className="truncate">{me.email}</span>}
          </span>
        </div>
      </Card>

      <Section
        title="VATUSA membership"
        icon={Building2}
        note={
          v?.synced_at && (
            <span className="flex items-center gap-1.5">
              <RefreshCw className="size-3" />
              Last synced <span className="font-mono">{fmtDate(v.synced_at)}</span>
            </span>
          )
        }
      >
        {!v ? (
          <p className="text-sm text-ink-2">
            Your VATUSA details haven’t synced yet. They’ll appear here after your next sign-in once VATUSA
            sync is enabled.
          </p>
        ) : (
          <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
            <Field
              label="Home facility"
              value={
                v.home_facility ? (
                  <>
                    <span className="font-mono">{v.home_facility}</span>
                    {homeFacName && <span className="font-normal text-ink-2"> — {homeFacName}</span>}
                  </>
                ) : (
                  "—"
                )
              }
            />
            <Field label="Rating" value={me.rating ?? "—"} mono />
            <Field label="Home controller" value={v.home_controller ? "Yes" : "No"} />
            <Field
              label="Member since"
              value={
                <span className="inline-flex items-center gap-1.5">
                  <CalendarDays className="size-3.5 text-ink-3" />
                  <span className="font-mono text-[13px]">{fmtDate(v.facility_join)}</span>
                </span>
              }
            />
          </div>
        )}
      </Section>

      {rolesByFacility.length > 0 && (
        <Section title="VATUSA roles" icon={ShieldCheck}>
          <div className="flex flex-col">
            {rolesByFacility.map(([facility, roles]) => (
              <div key={facility} className="flex items-center gap-3 border-b border-line-soft py-2 last:border-b-0">
                <span
                  className="w-24 shrink-0 font-mono text-[13px] font-semibold text-ink"
                  title={facName(facility) ?? undefined}
                >
                  {facility}
                </span>
                <div className="flex flex-wrap gap-1.5">
                  {roles.map((r) => (
                    <StatusPill key={r}>{r}</StatusPill>
                  ))}
                </div>
              </div>
            ))}
          </div>
        </Section>
      )}

      {v && v.visits.length > 0 && (
        <Section title="Visiting facilities" icon={MapPin}>
          <div className="flex flex-wrap gap-1.5">
            {v.visits.map((f) => (
              <span key={f} title={facName(f) ?? undefined}>
                <StatusPill className="font-mono">{f}</StatusPill>
              </span>
            ))}
          </div>
        </Section>
      )}

      <DiscordCard />

      <Section title="OIS access" icon={KeyRound}>
        <div className="flex flex-wrap gap-1.5">
          {me.server_admin && <StatusPill tone="brand">Server admin</StatusPill>}
          {oisRoles.map((r) => (
            <StatusPill key={r}>{r}</StatusPill>
          ))}
          {!me.server_admin && oisRoles.length === 0 && <span className="text-sm text-ink-2">Standard member</span>}
        </div>
      </Section>
    </div>
  );
}
