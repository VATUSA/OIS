import {useState} from "react";
import {Button, ConfirmButton} from "@ois/ui";
import {GripVertical, PictureInPicture2, RefreshCw, X} from "lucide-react";

import {can} from "@/lib/platform";

import {type WidgetStatus, WidgetStatusReporter} from "./widget-status";

/** "Updated 12s ago" style label for a data widget's last successful load. */
function agoLabel(ts: number): string {
  if (!ts) return "Not loaded yet";
  const s = Math.round((Date.now() - ts) / 1000);
  if (s < 5) return "Updated just now";
  if (s < 60) return `Updated ${s}s ago`;
  const m = Math.round(s / 60);
  return m < 60 ? `Updated ${m}m ago` : `Updated ${Math.round(m / 60)}h ago`;
}

/** Refresh button + last-updated tooltip, shown whenever the body reports data status. */
function RefreshControl({ status }: { status: WidgetStatus }) {
  return (
    <button
      type="button"
      onClick={status.refetch}
      disabled={status.isFetching}
      title={status.isFetching ? "Refreshing…" : `${agoLabel(status.updatedAt)} · click to refresh`}
      aria-label="Refresh data"
      className="text-ink-3 transition-colors hover:text-ink disabled:opacity-60"
    >
      <RefreshCw className={"size-3.5 " + (status.isFetching ? "animate-spin" : "")} />
    </button>
  );
}

/**
 * Chrome around every widget. In the default "full" mode a header carries the drag handle, title,
 * a data-refresh control (when the body reports one), and remove. In `bare` mode (text/dividers)
 * the body renders edge-to-edge with only a small floating drag/remove overlay while editing.
 * The drag handle carries `.widget-drag-handle`, the grid's drag target, so the body stays
 * interactive; `draggable=false` (mobile stack) hides it.
 */
export function WidgetFrame({
  title,
  editing,
  onRemove,
  onPopOut,
  flush = false,
  bare = false,
  draggable = true,
  children,
}: {
  title: string;
  editing: boolean;
  onRemove: () => void;
  /** Float this panel into its own always-on-top window. Desktop only (#349); omit elsewhere. */
  onPopOut?: () => void;
  /** Render the body edge-to-edge with no padding/scroll (for the map/chart). */
  flush?: boolean;
  /** Headerless presentational widget (text, divider) — chrome collapses to a hover overlay. */
  bare?: boolean;
  /** Show the drag handle (false in the non-draggable mobile stack). */
  draggable?: boolean;
  children: React.ReactNode;
}) {
  const [status, setStatus] = useState<WidgetStatus | null>(null);
  const body = <WidgetStatusReporter value={setStatus}>{children}</WidgetStatusReporter>;

  if (bare) {
    return (
      <div className="group relative h-full">
        {editing && (
          <div className="absolute right-1 top-1 z-10 flex items-center gap-1 rounded-xs border border-line bg-panel px-1 py-0.5 opacity-0 transition-opacity group-hover:opacity-100">
            {draggable && (
              <span
                className="widget-drag-handle cursor-move text-ink-3 hover:text-ink"
                title="Drag to move"
              >
                <GripVertical className="size-4" />
              </span>
            )}
            <ConfirmButton
              size="icon"
              variant="ghost"
              className="size-6 text-ink-3 hover:text-danger"
              warn="Remove this widget?"
              onConfirm={onRemove}
            >
              <X className="size-3.5" />
            </ConfirmButton>
          </div>
        )}
        {body}
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col overflow-hidden rounded-md border border-line bg-card">
      <div className="flex items-center gap-2 border-b border-line px-3 py-1.5">
        {editing && draggable && (
          <span
            className="widget-drag-handle -ml-1 cursor-move text-ink-3 hover:text-ink"
            title="Drag to move"
          >
            <GripVertical className="size-4" />
          </span>
        )}
        <span className="truncate text-sm font-semibold">{title}</span>
        <div className="ml-auto flex items-center gap-1.5">
          {status && <RefreshControl status={status} />}
          {onPopOut && can("miniWindows") && (
            <Button
              size="icon"
              variant="ghost"
              className="size-7 text-ink-3 hover:text-ink"
              title="Pop out into a floating window"
              onClick={onPopOut}
            >
              <PictureInPicture2 className="size-4" />
            </Button>
          )}
          {editing && (
            <ConfirmButton
              size="icon"
              variant="ghost"
              className="size-7 text-ink-3 hover:text-danger"
              warn="Remove this widget?"
              onConfirm={onRemove}
            >
              <X className="size-4" />
            </ConfirmButton>
          )}
        </div>
      </div>
      <div className={"min-h-0 flex-1 " + (flush ? "overflow-hidden" : "overflow-auto p-3")}>
        {body}
      </div>
    </div>
  );
}
