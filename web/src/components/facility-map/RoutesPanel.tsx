import {useState} from "react";
import {Button, ConfirmButton} from "@ois/ui";
import {Pencil, Plus, Trash2, X} from "lucide-react";

import {
  RouteEditor,
  blankRouteForm,
  routeFormFrom,
  type RouteForm,
} from "@/components/map/fca/editors";
import {hasPermission} from "@/lib/permissions";
import {useMe} from "@/lib/auth";
import {
  useCreateRoute,
  useDeleteRoute,
  useRoutes,
  useUpdateRoute,
  type UpsertRoute,
} from "@/lib/route";

/**
 * Manage a facility's routes from its map. New/edited routes are stamped with this ARTCC (the server
 * facility-scopes editing to it); global routes are listed read-only since they're managed on the flow
 * map. A route is just a filed-route string the nav engine resolves — no map drawing needed.
 */
export function FacilityRoutesPanel({
  facilityId,
  onClose,
}: {
  facilityId: string;
  onClose: () => void;
}) {
  const { data: me } = useMe();
  const canDelete = hasPermission(me, "flow.route.delete");
  const routes = useRoutes(facilityId);
  const create = useCreateRoute();
  const update = useUpdateRoute();
  const remove = useDeleteRoute();
  const [form, setForm] = useState<RouteForm | null>(null);

  const own = (routes.data ?? []).filter((r) => r.artcc === facilityId);
  const global = (routes.data ?? []).filter((r) => !r.artcc);

  const save = () => {
    if (!form || !form.name.trim() || !form.route.trim()) return;
    const body: UpsertRoute = {
      name: form.name.trim(),
      color: form.color,
      artcc: facilityId,
      route: form.route.trim().toUpperCase(),
      dep: form.dep.trim().toUpperCase(),
      arr: form.arr.trim().toUpperCase(),
    };
    const done = () => setForm(null);
    if (form.id) update.mutate({ id: form.id, body }, { onSuccess: done });
    else create.mutate(body, { onSuccess: done });
  };

  return (
    <div className="absolute right-3 top-3 z-[600] flex max-h-[calc(100%-1.5rem)] w-80 flex-col overflow-hidden rounded-lg border bg-background/95 shadow-xl backdrop-blur">
      <div className="flex items-center justify-between border-b px-3 py-2">
        <span className="text-sm font-semibold">Routes · {facilityId}</span>
        <button
          type="button"
          onClick={onClose}
          title="Close"
          className="rounded p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
        >
          <X className="size-4" />
        </button>
      </div>

      {form ? (
        <RouteEditor
          form={form}
          onChange={setForm}
          onSave={save}
          onCancel={() => setForm(null)}
          saving={create.isPending || update.isPending}
        />
      ) : (
        <div className="flex flex-col gap-2 overflow-y-auto p-3">
          <Button size="sm" className="self-start" onClick={() => setForm(blankRouteForm(own.length))}>
            <Plus className="mr-1 size-4" /> New route
          </Button>

          {own.length === 0 ? (
            <p className="py-2 text-center text-xs text-muted-foreground">
              No routes for {facilityId} yet.
            </p>
          ) : (
            <ul className="flex flex-col gap-1">
              {own.map((r) => (
                <li key={r.id} className="flex items-center gap-2 rounded-md border bg-muted/20 p-2">
                  <span
                    className="size-3 shrink-0 rounded-full"
                    style={{ backgroundColor: r.color }}
                  />
                  <span className="flex-1 truncate text-sm">{r.name || "(unnamed)"}</span>
                  <button
                    type="button"
                    onClick={() => setForm(routeFormFrom(r))}
                    title="Edit route"
                    className="rounded p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
                  >
                    <Pencil className="size-3.5" />
                  </button>
                  {canDelete && (
                    <ConfirmButton
                      size="icon"
                      variant="ghost"
                      warn="Delete this route?"
                      onConfirm={() => remove.mutate(r.id)}
                      title="Delete route"
                    >
                      <Trash2 className="size-3.5" />
                    </ConfirmButton>
                  )}
                </li>
              ))}
            </ul>
          )}

          {global.length > 0 && (
            <div className="mt-1 border-t pt-2">
              <div className="mb-1 text-xs font-medium text-muted-foreground">
                Global routes · edit on the flow map
              </div>
              <ul className="flex flex-col gap-0.5">
                {global.map((r) => (
                  <li
                    key={r.id}
                    className="flex items-center gap-2 px-1 py-0.5 text-sm text-muted-foreground"
                  >
                    <span
                      className="size-2.5 shrink-0 rounded-full"
                      style={{ backgroundColor: r.color }}
                    />
                    <span className="truncate">{r.name || "(unnamed)"}</span>
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
