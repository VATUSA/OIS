import * as React from "react";

import {Button} from "./button";
import {Input} from "./input";

export type ConfirmOptions = {
  title: string;
  description?: string;
  confirmText?: string;
  cancelText?: string;
  /** Style the confirm button as a destructive action (red). */
  destructive?: boolean;
};

export type PromptOptions = ConfirmOptions & {
  /** Field label above the input. */
  label?: string;
  placeholder?: string;
  defaultValue?: string;
};

type Request =
  | { kind: "confirm"; opts: ConfirmOptions; resolve: (v: boolean) => void }
  | { kind: "prompt"; opts: PromptOptions; resolve: (v: string | null) => void };

type DialogContextValue = {
  confirm: (opts: ConfirmOptions) => Promise<boolean>;
  prompt: (opts: PromptOptions) => Promise<string | null>;
};

const DialogContext = React.createContext<DialogContextValue | null>(null);

/**
 * App-level provider for our own confirm/prompt modals — a drop-in replacement for the
 * browser's `window.confirm` / `window.prompt`. Mount once near the app root.
 */
export function DialogProvider({ children }: { children: React.ReactNode }) {
  const [request, setRequest] = React.useState<Request | null>(null);
  const [value, setValue] = React.useState("");
  const inputRef = React.useRef<HTMLInputElement>(null);
  const confirmRef = React.useRef<HTMLButtonElement>(null);

  const confirm = React.useCallback(
    (opts: ConfirmOptions) =>
      new Promise<boolean>((resolve) => setRequest({ kind: "confirm", opts, resolve })),
    [],
  );
  const prompt = React.useCallback(
    (opts: PromptOptions) =>
      new Promise<string | null>((resolve) => {
        setValue(opts.defaultValue ?? "");
        setRequest({ kind: "prompt", opts, resolve });
      }),
    [],
  );

  const cancel = React.useCallback(() => {
    setRequest((cur) => {
      if (cur) cur.resolve((cur.kind === "confirm" ? false : null) as never);
      return null;
    });
  }, []);

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    setRequest((cur) => {
      if (cur) cur.resolve((cur.kind === "confirm" ? true : value.trim()) as never);
      return null;
    });
  };

  // Escape cancels; focus the input (prompt) or the confirm button (confirm) on open.
  React.useEffect(() => {
    if (!request) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") cancel();
    };
    window.addEventListener("keydown", onKey);
    const raf = requestAnimationFrame(() => {
      if (request.kind === "prompt") inputRef.current?.focus();
      else confirmRef.current?.focus();
    });
    return () => {
      window.removeEventListener("keydown", onKey);
      cancelAnimationFrame(raf);
    };
  }, [request, cancel]);

  const ctx = React.useMemo(() => ({ confirm, prompt }), [confirm, prompt]);

  return (
    <DialogContext.Provider value={ctx}>
      {children}
      {request && (
        <div className="fixed inset-0 z-[110] flex items-center justify-center p-4">
          <div
            className="absolute inset-0 bg-black/50 backdrop-blur-[1px]"
            onClick={cancel}
            aria-hidden="true"
          />
          <form
            onSubmit={submit}
            role="dialog"
            aria-modal="true"
            aria-label={request.opts.title}
            className="relative w-full max-w-sm rounded-lg border bg-background p-5 shadow-xl"
          >
            <h2 className="text-base font-semibold text-foreground">
              {request.opts.title}
            </h2>
            {request.opts.description && (
              <p className="mt-1.5 text-sm text-muted-foreground">
                {request.opts.description}
              </p>
            )}
            {request.kind === "prompt" && (
              <div className="mt-3 flex flex-col gap-1">
                {request.opts.label && (
                  <label className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                    {request.opts.label}
                  </label>
                )}
                <Input
                  ref={inputRef}
                  value={value}
                  placeholder={request.opts.placeholder}
                  onChange={(e) => setValue(e.target.value)}
                />
              </div>
            )}
            <div className="mt-5 flex justify-end gap-2">
              <Button type="button" variant="ghost" onClick={cancel}>
                {request.opts.cancelText ?? "Cancel"}
              </Button>
              <Button
                ref={confirmRef}
                type="submit"
                variant={request.opts.destructive ? "destructive" : "default"}
                disabled={request.kind === "prompt" && !value.trim()}
              >
                {request.opts.confirmText ??
                  (request.kind === "prompt" ? "Save" : "Confirm")}
              </Button>
            </div>
          </form>
        </div>
      )}
    </DialogContext.Provider>
  );
}

function useDialog(): DialogContextValue {
  const ctx = React.useContext(DialogContext);
  if (!ctx) {
    throw new Error("useConfirm/usePrompt must be used within a DialogProvider");
  }
  return ctx;
}

/** Imperative confirm: `if (await confirm({ title, destructive: true })) { … }`. */
export function useConfirm() {
  return useDialog().confirm;
}

/** Imperative prompt: `const name = await prompt({ title, label }); if (name) { … }`. */
export function usePrompt() {
  return useDialog().prompt;
}
