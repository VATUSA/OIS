import {useMemo} from "react";

import {Avatar, AvatarFallback, Badge, Card, CardContent, CardDescription, CardHeader, CardTitle,} from "@ois/ui";
import {Building2, CalendarDays, RefreshCw} from "lucide-react";

import {useMe} from "@/lib/auth";
import {useFacilities} from "@/lib/admin";

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

function Field({label, value}: {label: string; value: React.ReactNode}) {
  return (
    <div className="flex flex-col gap-0.5">
      <span className="text-xs uppercase tracking-wide text-muted-foreground">
        {label}
      </span>
      <span className="text-sm font-medium">{value}</span>
    </div>
  );
}

export function ProfilePage() {
  const {data: me, isLoading} = useMe();
  const {data: facilities} = useFacilities();

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

  if (isLoading) {
    return (
      <p className="py-10 text-center text-sm text-muted-foreground">Loading…</p>
    );
  }
  if (!me) {
    return (
      <p className="py-10 text-center text-sm text-muted-foreground">
        Sign in to view your profile.
      </p>
    );
  }

  const oisRoles = me.role_names.filter((r) => r !== "USER");
  const homeFacName = v?.home_facility ? facName(v.home_facility) : null;

  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-6">
      <div className="flex items-center gap-4">
        <Avatar className="size-14 text-lg">
          <AvatarFallback>{initials(me.display_name)}</AvatarFallback>
        </Avatar>
        <div>
          <h1 className="text-2xl font-semibold">{me.display_name}</h1>
          <p className="text-sm text-muted-foreground">
            CID {me.cid}
            {me.rating ? ` · ${me.rating}` : ""}
            {me.email ? ` · ${me.email}` : ""}
          </p>
        </div>
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Building2 className="size-4 text-muted-foreground" />
            VATUSA membership
          </CardTitle>
          {v?.synced_at && (
            <CardDescription className="flex items-center gap-1.5">
              <RefreshCw className="size-3" />
              Last synced {fmtDate(v.synced_at)}
            </CardDescription>
          )}
        </CardHeader>
        <CardContent>
          {!v ? (
            <p className="text-sm text-muted-foreground">
              Your VATUSA details haven’t synced yet. They’ll appear here after
              your next sign-in once VATUSA sync is enabled.
            </p>
          ) : (
            <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
              <Field
                label="Home facility"
                value={
                  v.home_facility
                    ? homeFacName
                      ? `${v.home_facility} — ${homeFacName}`
                      : v.home_facility
                    : "—"
                }
              />
              <Field label="Rating" value={me.rating ?? "—"} />
              <Field
                label="Home controller"
                value={v.home_controller ? "Yes" : "No"}
              />
              <Field
                label="Member since"
                value={
                  <span className="inline-flex items-center gap-1">
                    <CalendarDays className="size-3.5 text-muted-foreground" />
                    {fmtDate(v.facility_join)}
                  </span>
                }
              />
            </div>
          )}
        </CardContent>
      </Card>

      {rolesByFacility.length > 0 && (
        <Card>
          <CardHeader>
            <CardTitle>VATUSA roles</CardTitle>
          </CardHeader>
          <CardContent className="flex flex-col gap-2.5">
            {rolesByFacility.map(([facility, roles]) => (
              <div key={facility} className="flex items-center gap-3">
                <span
                  className="w-24 shrink-0 font-mono text-sm font-semibold"
                  title={facName(facility) ?? undefined}
                >
                  {facility}
                </span>
                <div className="flex flex-wrap gap-1.5">
                  {roles.map((r) => (
                    <Badge key={r} variant="secondary">
                      {r}
                    </Badge>
                  ))}
                </div>
              </div>
            ))}
          </CardContent>
        </Card>
      )}

      {v && v.visits.length > 0 && (
        <Card>
          <CardHeader>
            <CardTitle>Visiting facilities</CardTitle>
          </CardHeader>
          <CardContent className="flex flex-wrap gap-1.5">
            {v.visits.map((f) => (
              <Badge key={f} variant="outline" title={facName(f) ?? undefined}>
                {f}
              </Badge>
            ))}
          </CardContent>
        </Card>
      )}

      <Card>
        <CardHeader>
          <CardTitle>OIS access</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-wrap gap-1.5">
          {me.server_admin && <Badge>Server admin</Badge>}
          {oisRoles.map((r) => (
            <Badge key={r} variant="secondary">
              {r}
            </Badge>
          ))}
          {!me.server_admin && oisRoles.length === 0 && (
            <span className="text-sm text-muted-foreground">
              Standard member
            </span>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
