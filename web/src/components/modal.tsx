import {useEffect} from "react";
import {createPortal} from "react-dom";
import {X} from "lucide-react";

/**
 * A minimal centered modal dialog. `@ois/ui` only ships imperative confirm/prompt dialogs, so this
 * hosts arbitrary form content (e.g. the DCC / ACE panels). Portaled to `document.body` so it
 * escapes any transformed ancestor; closes on overlay click or Escape.
 */
export function Modal({
  open,
  onClose,
  title,
  children,
  size = "md",
}: {
  open: boolean;
  onClose: () => void;
  title: React.ReactNode;
  children: React.ReactNode;
  size?: "md" | "lg";
}) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  return createPortal(
    <div className="fixed inset-0 z-[1000] flex items-center justify-center p-4" role="dialog" aria-modal="true">
      <div className="absolute inset-0 bg-black/50" onClick={onClose} />
      <div
        className={
          "relative z-10 flex max-h-[85vh] w-full flex-col overflow-hidden rounded-lg border bg-card shadow-2xl " +
          (size === "lg" ? "max-w-2xl" : "max-w-lg")
        }
      >
        <div className="flex items-center justify-between gap-2 border-b px-4 py-3">
          <h2 className="text-sm font-semibold">{title}</h2>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="text-muted-foreground transition-colors hover:text-foreground"
          >
            <X className="size-4" />
          </button>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto p-4">{children}</div>
      </div>
    </div>,
    document.body,
  );
}
