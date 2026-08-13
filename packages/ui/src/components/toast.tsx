import * as React from "react";
import {AlertTriangle, CheckCircle2, Info, X, XCircle,} from "lucide-react";

import {cn} from "../lib/utils";

export type ToastVariant = "success" | "error" | "warning" | "info";

export type ToastOptions = {
  description?: string;
  /** Auto-dismiss delay in ms. Defaults to 4000 (6000 for errors). */
  duration?: number;
};

type Toast = ToastOptions & {
  id: string;
  variant: ToastVariant;
  title: string;
};

type ToastContextValue = {
  push: (t: Omit<Toast, "id">) => void;
};

const ToastContext = React.createContext<ToastContextValue | null>(null);

const VARIANTS: Record<
  ToastVariant,
  { icon: React.ComponentType<{ className?: string }>; accent: string; icon_color: string }
> = {
  success: { icon: CheckCircle2, accent: "border-l-emerald-500", icon_color: "text-emerald-500" },
  error: { icon: XCircle, accent: "border-l-destructive", icon_color: "text-destructive" },
  warning: { icon: AlertTriangle, accent: "border-l-amber-500", icon_color: "text-amber-500" },
  info: { icon: Info, accent: "border-l-sky-500", icon_color: "text-sky-500" },
};

function ToastCard({ t, onDismiss }: { t: Toast; onDismiss: (id: string) => void }) {
  const [shown, setShown] = React.useState(false);
  const timer = React.useRef<number | undefined>(undefined);
  const { icon: Icon, accent, icon_color } = VARIANTS[t.variant];

  const close = React.useCallback(() => {
    window.clearTimeout(timer.current);
    setShown(false);
    window.setTimeout(() => onDismiss(t.id), 180);
  }, [t.id, onDismiss]);

  React.useEffect(() => {
    const raf = requestAnimationFrame(() => setShown(true));
    const delay = t.duration ?? (t.variant === "error" ? 6000 : 4000);
    timer.current = window.setTimeout(close, delay);
    return () => {
      cancelAnimationFrame(raf);
      window.clearTimeout(timer.current);
    };
  }, [close, t.duration, t.variant]);

  return (
    <div
      role="status"
      className={cn(
        "pointer-events-auto flex w-80 max-w-[90vw] items-start gap-3 rounded-lg border border-l-4 bg-background p-3 shadow-lg transition-all duration-200",
        accent,
        shown ? "translate-x-0 opacity-100" : "translate-x-4 opacity-0",
      )}
    >
      <Icon className={cn("mt-0.5 size-5 shrink-0", icon_color)} />
      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium text-foreground">{t.title}</p>
        {t.description && (
          <p className="mt-0.5 text-sm text-muted-foreground">{t.description}</p>
        )}
      </div>
      <button
        type="button"
        aria-label="Dismiss"
        onClick={close}
        className="shrink-0 text-muted-foreground transition-colors hover:text-foreground"
      >
        <X className="size-4" />
      </button>
    </div>
  );
}

export function ToastProvider({ children }: { children: React.ReactNode }) {
  const [toasts, setToasts] = React.useState<Toast[]>([]);

  const push = React.useCallback((t: Omit<Toast, "id">) => {
    const id =
      typeof crypto !== "undefined" && crypto.randomUUID
        ? crypto.randomUUID()
        : `${Date.now()}-${Math.random()}`;
    setToasts((prev) => [...prev, { id, ...t }]);
  }, []);

  const dismiss = React.useCallback((id: string) => {
    setToasts((prev) => prev.filter((x) => x.id !== id));
  }, []);

  return (
    <ToastContext.Provider value={{ push }}>
      {children}
      <div className="pointer-events-none fixed bottom-4 right-4 z-[100] flex flex-col gap-2">
        {toasts.map((t) => (
          <ToastCard key={t.id} t={t} onDismiss={dismiss} />
        ))}
      </div>
    </ToastContext.Provider>
  );
}

export type ToastApi = {
  success: (title: string, opts?: ToastOptions) => void;
  error: (title: string, opts?: ToastOptions) => void;
  warning: (title: string, opts?: ToastOptions) => void;
  info: (title: string, opts?: ToastOptions) => void;
};

/** Access the toast API. Must be used within a `ToastProvider`. */
export function useToast(): ToastApi {
  const ctx = React.useContext(ToastContext);
  if (!ctx) {
    throw new Error("useToast must be used within a ToastProvider");
  }
  return React.useMemo(() => {
    const make =
      (variant: ToastVariant) => (title: string, opts?: ToastOptions) =>
        ctx.push({ variant, title, ...opts });
    return {
      success: make("success"),
      error: make("error"),
      warning: make("warning"),
      info: make("info"),
    };
  }, [ctx]);
}
