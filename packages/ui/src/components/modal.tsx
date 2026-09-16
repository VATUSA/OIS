import * as React from "react";
import {createPortal} from "react-dom";
import {X} from "lucide-react";

import {cn} from "../lib/utils";

const SIZE = { sm: "max-w-sm", md: "max-w-lg", lg: "max-w-2xl", xl: "max-w-4xl" } as const;

export type ModalProps = {
  open: boolean;
  onClose: () => void;
  /** Header title; omit for a headerless panel (e.g. a command palette). */
  title?: React.ReactNode;
  description?: React.ReactNode;
  size?: keyof typeof SIZE;
  /** Sticks to the bottom of the panel, outside the scrolling body. */
  footer?: React.ReactNode;
  /** `center` (default); `top` pins it high (pickers, palettes); `right` is a full-height side drawer. */
  placement?: "center" | "top" | "right";
  className?: string;
  "aria-label"?: string;
  children: React.ReactNode;
};

/**
 * The one overlay: a portaled, centred panel on a dimmed ground. Closes on Escape or a click on the
 * ground. Depth is a surface step + hairline, never a shadow (DESIGN.md rule 3).
 */
export function Modal({
  open,
  onClose,
  title,
  description,
  size = "md",
  footer,
  placement = "center",
  className,
  children,
  ...aria
}: ModalProps) {
  React.useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  return createPortal(
    <div
      className={cn(
        "fixed inset-0 z-[1000] flex",
        placement === "center" && "items-center justify-center p-4",
        placement === "top" && "items-start justify-center p-4 pt-[12vh]",
        placement === "right" && "justify-end",
      )}
      role="dialog"
      aria-modal="true"
      aria-label={aria["aria-label"] ?? (typeof title === "string" ? title : undefined)}
    >
      <div className="absolute inset-0 bg-ground/70" onClick={onClose} aria-hidden="true" />
      <div
        className={cn(
          "relative flex w-full flex-col overflow-hidden border-line bg-panel",
          placement === "right"
            ? "h-full max-w-[90vw] border-l sm:max-w-sm"
            : cn("max-h-[85vh] rounded-lg border", SIZE[size]),
          className,
        )}
      >
        {title != null && (
          <div className="flex items-start justify-between gap-3 border-b border-line px-5 py-4">
            <div className="min-w-0">
              <h2 className="text-base font-bold text-ink">{title}</h2>
              {description && <p className="mt-1 text-sm text-ink-2">{description}</p>}
            </div>
            <button
              type="button"
              onClick={onClose}
              aria-label="Close"
              className="rounded-xs p-1 text-ink-3 transition-colors hover:bg-panel-2 hover:text-ink"
            >
              <X className="size-4" />
            </button>
          </div>
        )}
        <div className={cn("min-h-0 flex-1 overflow-y-auto", title != null && "p-5")}>{children}</div>
        {footer && (
          <div className="flex items-center justify-end gap-2 border-t border-line px-5 py-3">{footer}</div>
        )}
      </div>
    </div>,
    document.body,
  );
}
