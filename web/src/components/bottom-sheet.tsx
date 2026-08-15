import {useEffect, useRef, useState} from "react";
import {X} from "lucide-react";

/** True on phone-width viewports (matches the Tailwind `md` breakpoint). */
export function useIsMobile() {
  const [mobile, setMobile] = useState(
    () => window.matchMedia("(max-width: 767px)").matches,
  );
  useEffect(() => {
    const mq = window.matchMedia("(max-width: 767px)");
    const on = () => setMobile(mq.matches);
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, []);
  return mobile;
}

/**
 * A panel that is a normal side column on desktop but a **drag-to-resize bottom
 * sheet** on mobile — keeping whatever is behind it (a map, a board) visible
 * above. `open` controls mobile visibility (always shown on desktop); the top
 * grab handle resizes the sheet between 120px and 85vh.
 */
export function BottomSheet({
  open = true,
  onClose,
  desktopClassName = "",
  initialFraction = 0.5,
  children,
}: {
  /** Mobile only — whether the sheet is slid up. Desktop always renders. */
  open?: boolean;
  /** Shows a close (X) on mobile; also let callers dismiss. */
  onClose?: () => void;
  /** Classes applied in both modes (desktop width/border live here). */
  desktopClassName?: string;
  /** Starting height as a fraction of the viewport. */
  initialFraction?: number;
  children: React.ReactNode;
}) {
  const isMobile = useIsMobile();
  const [height, setHeight] = useState(() =>
    Math.round(window.innerHeight * initialFraction),
  );
  const drag = useRef<{ startY: number; startH: number } | null>(null);

  const onDown = (e: React.PointerEvent) => {
    drag.current = { startY: e.clientY, startH: height };
    try {
      e.currentTarget.setPointerCapture(e.pointerId);
    } catch {
      /* capture unsupported — move events still fire */
    }
  };
  const onMove = (e: React.PointerEvent) => {
    if (!drag.current) return;
    const dy = drag.current.startY - e.clientY; // drag up ⇒ taller
    const max = Math.round(window.innerHeight * 0.85);
    setHeight(Math.min(max, Math.max(120, drag.current.startH + dy)));
  };
  const onUp = (e: React.PointerEvent) => {
    drag.current = null;
    try {
      e.currentTarget.releasePointerCapture(e.pointerId);
    } catch {
      /* already released */
    }
  };

  return (
    <div
      className={
        "relative flex flex-col bg-background " +
        desktopClassName +
        " max-md:absolute max-md:inset-x-0 max-md:bottom-0 max-md:z-[700] max-md:w-full" +
        " max-md:rounded-t-2xl max-md:border max-md:border-b-0 max-md:shadow-2xl" +
        " max-md:transition-transform " +
        (open
          ? "max-md:translate-y-0"
          : "max-md:pointer-events-none max-md:translate-y-full")
      }
      style={isMobile ? { height } : undefined}
    >
      {/* Grab handle — resize the sheet on mobile. */}
      <div
        onPointerDown={onDown}
        onPointerMove={onMove}
        onPointerUp={onUp}
        className="flex shrink-0 cursor-row-resize touch-none items-center justify-center pb-1 pt-2 md:hidden"
      >
        <span className="h-1.5 w-10 rounded-full bg-muted-foreground/40" />
      </div>
      {onClose && (
        <button
          type="button"
          aria-label="Close"
          onClick={onClose}
          className="absolute right-2 top-1.5 z-10 rounded p-1 text-muted-foreground hover:text-foreground md:hidden"
        >
          <X className="size-4" />
        </button>
      )}
      {children}
    </div>
  );
}
