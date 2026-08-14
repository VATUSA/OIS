import {useState} from "react";
import {Badge, Button, Card, CardContent, ConfirmButton, Input, useConfirm} from "@ois/ui";
import {Layers, Play, Plus, X} from "lucide-react";

import {useMe} from "@/lib/auth";
import {
  type TmiPackage,
  type TmiPackageItem,
  useActivatePackage,
  useAddPackageItem,
  useCreatePackage,
  useDeletePackage,
  useDeletePackageItem,
  usePackages,
} from "@/lib/events";
import {hasPermission} from "@/lib/permissions";
import {formatZulu, parseZulu} from "@/lib/time";

type Kind = "program" | "restriction" | "ground_stop";

const KINDS: { value: Kind; label: string }[] = [
  { value: "program", label: "Program" },
  { value: "restriction", label: "Restriction" },
  { value: "ground_stop", label: "Ground stop" },
];

const kindLabel = (k: string) => KINDS.find((x) => x.value === k)?.label ?? k;

/** One-line summary of an item from its kind + payload. */
function itemSummary(item: TmiPackageItem): string {
  const p = item.payload as unknown as Record<string, unknown>;
  const s = (k: string) => (p[k] == null ? "" : String(p[k]));
  if (item.kind === "program") {
    const extra = [
      Number(p.trail) > 0 ? `${p.trail} MIN` : "",
      Number(p.mit) > 0 ? `${p.mit} MIT` : "",
    ]
      .filter(Boolean)
      .join(" · ");
    return `${s("icao")} · AAR ${s("aar")}${extra ? ` · ${extra}` : ""}`;
  }
  if (item.kind === "restriction") {
    return `${s("requesting")} → ${s("providing")} · ${s("restriction")}`;
  }
  return `${s("airport")} · ${s("scope") || "all"} · ${s("until") ? `${s("until")}z` : "UFN"}`;
}

function AddItemForm({
  eventId,
  packageId,
}: {
  eventId: number;
  packageId: string;
}) {
  const add = useAddPackageItem(eventId);
  const [kind, setKind] = useState<Kind>("program");
  const [f, setF] = useState<Record<string, string>>({});
  const set = (k: string, v: string) => setF((prev) => ({ ...prev, [k]: v }));

  function submit() {
    let payload: Record<string, unknown> | null = null;
    if (kind === "program") {
      if (!f.icao || !f.aar) return;
      payload = {
        icao: f.icao,
        aar: Number(f.aar) || 0,
        trail: Number(f.trail) || 0,
        mit: Number(f.mit) || 0,
      };
    } else if (kind === "restriction") {
      if (!f.requesting || !f.providing || !f.restriction) return;
      payload = {
        requesting: f.requesting,
        providing: f.providing,
        restriction: f.restriction,
        start_time: parseZulu(f.start ?? "") ?? undefined,
        stop_time: parseZulu(f.stop ?? "") ?? undefined,
      };
    } else {
      if (!f.airport) return;
      payload = {
        airport: f.airport,
        scope: f.scope || undefined,
        until: f.until || undefined,
      };
    }
    add.mutate(
      { packageId, kind, payload },
      { onSuccess: () => setF({}) },
    );
  }

  const field = (key: string, placeholder: string, cls = "w-28") => (
    <Input
      className={cls}
      placeholder={placeholder}
      value={f[key] ?? ""}
      onChange={(e) => set(key, e.target.value)}
      onKeyDown={(e) => e.key === "Enter" && submit()}
    />
  );

  return (
    <div className="flex flex-col gap-2 rounded-md border border-dashed p-3">
      <div className="flex flex-wrap gap-1">
        {KINDS.map((k) => (
          <Button
            key={k.value}
            type="button"
            size="sm"
            variant={kind === k.value ? "default" : "secondary"}
            onClick={() => {
              setKind(k.value);
              setF({});
            }}
          >
            {k.label}
          </Button>
        ))}
      </div>
      <div className="flex flex-wrap items-center gap-2">
        {kind === "program" && (
          <>
            {field("icao", "ICAO", "w-24 font-mono uppercase")}
            {field("aar", "AAR", "w-16")}
            {field("trail", "trail", "w-16")}
            {field("mit", "MIT", "w-16")}
          </>
        )}
        {kind === "restriction" && (
          <>
            {field("requesting", "requesting", "w-28 font-mono uppercase")}
            {field("providing", "providing", "w-28 font-mono uppercase")}
            {field("restriction", "restriction (e.g. 20 MIT)", "w-48")}
            {field("start", "start DD/HHMMz", "w-32")}
            {field("stop", "stop DD/HHMMz", "w-32")}
          </>
        )}
        {kind === "ground_stop" && (
          <>
            {field("airport", "airport", "w-24 font-mono uppercase")}
            {field("scope", "scope (blank=all)", "w-36 uppercase")}
            {field("until", "until HHMM", "w-24")}
          </>
        )}
        <Button size="sm" onClick={submit} disabled={add.isPending}>
          <Plus />
          Add
        </Button>
      </div>
    </div>
  );
}

function PackageCard({
  eventId,
  pkg,
  canEdit,
}: {
  eventId: number;
  pkg: TmiPackage;
  canEdit: boolean;
}) {
  const del = useDeletePackage(eventId);
  const removeItem = useDeletePackageItem(eventId);
  const activate = useActivatePackage(eventId);
  const confirm = useConfirm();
  const draft = pkg.status === "draft";
  const editable = canEdit && draft;

  return (
    <div className="rounded-lg border">
      <div className="flex items-center justify-between gap-2 border-b px-4 py-2.5">
        <div className="flex items-center gap-2">
          <span className="font-semibold">{pkg.name}</span>
          <Badge variant={draft ? "secondary" : "success"}>{pkg.status}</Badge>
          {!draft && pkg.activated_at && (
            <span className="text-xs text-muted-foreground">
              activated {formatZulu(pkg.activated_at)}
            </span>
          )}
        </div>
        {canEdit && (
          <div className="flex items-center gap-2">
            {draft && (
              <Button
                size="sm"
                onClick={async () => {
                  if (pkg.items.length === 0) return;
                  const ok = await confirm({
                    title: `Activate “${pkg.name}”?`,
                    description: `This creates ${pkg.items.length} live TMU item(s).`,
                    confirmText: "Activate",
                  });
                  if (ok) activate.mutate(pkg.id);
                }}
                disabled={pkg.items.length === 0 || activate.isPending}
              >
                <Play />
                Activate
              </Button>
            )}
            <ConfirmButton
              size="icon"
              title="Delete package"
              aria-label="Delete package"
              onConfirm={() => del.mutate(pkg.id)}
              warn={`Delete the “${pkg.name}” package?`}
            >
              <X className="size-4" />
            </ConfirmButton>
          </div>
        )}
      </div>

      <div className="flex flex-col gap-3 p-4">
        {pkg.items.length === 0 ? (
          <p className="text-sm text-muted-foreground">No items yet.</p>
        ) : (
          <ul className="flex flex-col divide-y divide-border/60">
            {pkg.items.map((item) => (
              <li
                key={item.id}
                className="flex items-center justify-between gap-2 py-2 text-sm"
              >
                <span className="flex items-center gap-2">
                  <Badge variant="outline">{kindLabel(item.kind)}</Badge>
                  <span className="font-mono text-xs">{itemSummary(item)}</span>
                </span>
                {editable && (
                  <ConfirmButton
                    size="icon"
                    title="Remove item"
                    aria-label="Remove item"
                    onConfirm={() =>
                      removeItem.mutate({ packageId: pkg.id, itemId: item.id })
                    }
                    warn="Remove this item from the package?"
                  >
                    <X className="size-4" />
                  </ConfirmButton>
                )}
              </li>
            ))}
          </ul>
        )}

        {editable && <AddItemForm eventId={eventId} packageId={pkg.id} />}
      </div>
    </div>
  );
}

export function TmiPackagesSection({ eventId }: { eventId: number }) {
  const { data: me } = useMe();
  const canEdit = hasPermission(me, "events.plan.update");
  const packages = usePackages(eventId);
  const create = useCreatePackage(eventId);
  const [name, setName] = useState("");

  function add() {
    const n = name.trim();
    if (n) create.mutate(n, { onSuccess: () => setName("") });
  }

  const rows = packages.data ?? [];

  return (
    <Card>
      <CardContent className="flex flex-col gap-4 pt-6">
        <div className="flex items-center gap-2">
          <span className="flex size-8 items-center justify-center rounded-md bg-primary/10 text-primary">
            <Layers className="size-4" />
          </span>
          <div className="flex flex-col">
            <span className="font-semibold">TMI packages</span>
            <span className="text-xs text-muted-foreground">
              Draft the programs, restrictions, and ground stops, then activate
              them live for the event.
            </span>
          </div>
        </div>

        {canEdit && (
          <div className="flex flex-wrap items-center gap-2">
            <Input
              className="w-64"
              placeholder="New package name — e.g. FNO kickoff"
              value={name}
              onChange={(e) => setName(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && add()}
            />
            <Button onClick={add} disabled={create.isPending}>
              <Plus />
              New package
            </Button>
          </div>
        )}

        {!packages.data ? (
          <p className="py-2 text-sm text-muted-foreground">Loading…</p>
        ) : rows.length === 0 ? (
          <p className="py-2 text-sm text-muted-foreground">
            No TMI packages yet.
          </p>
        ) : (
          <div className="flex flex-col gap-4">
            {rows.map((pkg) => (
              <PackageCard
                key={pkg.id}
                eventId={eventId}
                pkg={pkg}
                canEdit={canEdit}
              />
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  );
}
