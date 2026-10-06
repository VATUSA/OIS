import {cn, ConfirmButton, Switch} from "@ois/ui";
import {useSortable} from "@dnd-kit/sortable";
import {CSS} from "@dnd-kit/utilities";
import {GripVertical, Pencil, Trash2} from "lucide-react";

import {useMe} from "@/lib/auth";
import type {Fca} from "@/lib/fca";
import {hasPermission} from "@/lib/permissions";
import {swatchCss} from "../lib/colors";

/** One draggable row in the sidebar FCA list (see #109 — order is per-viewer, via `usePersistedOrder`). */
export function FcaRow({
  fca,
  selected,
  count,
  canEdit,
  canDelete,
  onSelect,
  onToggleEnabled,
  onEdit,
  onDelete,
}: {
  fca: Fca;
  selected: boolean;
  count: number;
  canEdit: boolean;
  canDelete: boolean;
  onSelect: () => void;
  onToggleEnabled: () => void;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const { data: me } = useMe();
  // An event FCA is written only by its event's planners: the plain FCA routes refuse anyone else
  // (#736), so its toggle, Edit and Delete would only ever fail. The event builder already gates
  // `canEdit` on `events.plan.update`, so this only narrows the live map.
  const writable = fca.event_id == null || hasPermission(me, "events.plan.update");
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: fca.id,
  });
  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : fca.enabled ? 1 : 0.55,
  };
  return (
    <li
      ref={setNodeRef}
      style={style}
      className={cn("flex items-center gap-2 border-b border-line-soft px-3 py-2 text-sm", selected && "bg-brand-soft")}
    >
      <button
        type="button"
        className="cursor-grab text-ink-3 hover:text-ink"
        {...attributes}
        {...listeners}
      >
        <GripVertical className="size-3.5" />
      </button>
      <span
        className="size-3 shrink-0 rounded-full"
        style={{ background: swatchCss(fca.color) }}
        title={fca.enabled ? "Enabled" : "Disabled"}
      />
      <button type="button" onClick={onSelect} className="flex-1 truncate text-left font-mono">
        {fca.name}
        {fca.artcc && <span className="ml-1.5 text-xs text-ink-3">{fca.artcc}</span>}
      </button>
      <span
        className={cn(
          "shrink-0 rounded-full px-1.5 font-mono text-xs font-semibold",
          count > 0 ? "bg-brand-soft text-brand-ink" : "text-ink-3",
        )}
      >
        {count}
      </span>
      {canEdit && writable && (
        <span
          className="flex shrink-0 items-center"
          title={fca.enabled ? "Enabled — click to disable" : "Disabled — click to enable"}
        >
          <Switch
            checked={fca.enabled}
            onCheckedChange={onToggleEnabled}
            aria-label={`${fca.enabled ? "Disable" : "Enable"} the ${fca.name} FCA`}
            className="scale-[0.68]"
          />
        </span>
      )}
      {canEdit && writable && (
        <button type="button" title="Edit" onClick={onEdit} className="text-ink-3 hover:text-ink">
          <Pencil className="size-3.5" />
        </button>
      )}
      {canDelete && writable && (
        <ConfirmButton
          size="icon"
          className="size-7"
          title="Delete"
          aria-label="Delete FCA"
          onConfirm={onDelete}
          warn={`Delete the “${fca.name}” FCA?`}
        >
          <Trash2 className="size-3.5" />
        </ConfirmButton>
      )}
    </li>
  );
}
