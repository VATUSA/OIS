import * as React from "react";

import {Button} from "./button";
import {Input} from "./input";
import {Modal} from "./modal";

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

  // Focus the input (prompt) or the confirm button (confirm) on open. Modal handles Escape.
  React.useEffect(() => {
    if (!request) return;
    const raf = requestAnimationFrame(() => {
      if (request.kind === "prompt") inputRef.current?.focus();
      else confirmRef.current?.focus();
    });
    return () => cancelAnimationFrame(raf);
  }, [request]);

  const ctx = React.useMemo(() => ({ confirm, prompt }), [confirm, prompt]);

  return (
    <DialogContext.Provider value={ctx}>
      {children}
      <Modal
        open={request != null}
        onClose={cancel}
        size="sm"
        aria-label={request?.opts.title}
      >
        {request && (
          <form onSubmit={submit} className="p-5">
            <h2 className="text-base font-bold text-ink">{request.opts.title}</h2>
            {request.opts.description && (
              <p className="mt-1.5 text-sm text-ink-2">{request.opts.description}</p>
            )}
            {request.kind === "prompt" && (
              <div className="mt-3 flex flex-col gap-1">
                {request.opts.label && (
                  <label className="text-xs font-semibold uppercase tracking-wide text-ink-2">
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
        )}
      </Modal>
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
