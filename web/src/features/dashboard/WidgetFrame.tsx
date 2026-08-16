import {ConfirmButton} from "@ois/ui";
import {GripVertical, X} from "lucide-react";

/**
 * Chrome around every widget: a header (drag handle + title + remove) and a scrollable body.
 * The drag handle carries `.widget-drag-handle`, which the grid uses as its drag target so the
 * whole card isn't draggable (you can still interact with the body).
 */
export function WidgetFrame({
  title,
  editing,
  onRemove,
  flush = false,
  children,
}: {
  title: string;
  editing: boolean;
  onRemove: () => void;
  /** Render the body edge-to-edge with no padding/scroll (for the map). */
  flush?: boolean;
  children: React.ReactNode;
}) {
  return (
    <div className="flex h-full flex-col overflow-hidden rounded-lg border bg-card shadow-sm">
      <div className="flex items-center gap-2 border-b px-3 py-1.5">
        {editing && (
          <span
            className="widget-drag-handle -ml-1 cursor-move text-muted-foreground hover:text-foreground"
            title="Drag to move"
          >
            <GripVertical className="size-4" />
          </span>
        )}
        <span className="truncate text-sm font-medium">{title}</span>
        {editing && (
          <ConfirmButton
            size="icon"
            variant="ghost"
            className="ml-auto size-7 text-muted-foreground hover:text-destructive"
            warn="Remove this widget?"
            onConfirm={onRemove}
          >
            <X className="size-4" />
          </ConfirmButton>
        )}
      </div>
      <div className={"min-h-0 flex-1 " + (flush ? "overflow-hidden" : "overflow-auto p-3")}>
        {children}
      </div>
    </div>
  );
}
