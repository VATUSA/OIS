import * as React from "react";

import {cn} from "../lib/utils";
import {Button, type ButtonProps} from "./button";
import {useToast} from "./toast";

export type ConfirmButtonProps = Omit<ButtonProps, "onClick"> & {
  /** Runs on the second (confirming) click. */
  onConfirm: () => void;
  /** Warning toast shown when the button is first armed. */
  warn?: string;
  /** Sub-line of the warning toast. */
  warnDescription?: string;
  /** Milliseconds the armed state lasts before reverting. Default 5000. */
  timeoutMs?: number;
};

/**
 * A destructive button that confirms in place instead of via a browser popup: the first click
 * arms it (turns warning-toned + fires a warning toast), a second click within `timeoutMs` runs the
 * action (button flashes red), and no second click reverts it to its default look. Drop-in for
 * any delete/remove control — the surrounding mutation still shows the final success/error toast.
 */
export function ConfirmButton({
  onConfirm,
  warn,
  warnDescription = "Click again to confirm.",
  timeoutMs = 5000,
  children,
  className,
  variant = "ghost",
  size,
  disabled,
  ...rest
}: ConfirmButtonProps) {
  // A wrapping trigger (e.g. a tooltip via Slot) may inject its own onClick; run it alongside ours
  // instead of letting the spread below replace the arm/confirm handler.
  const { onClick: injectedClick, ...buttonProps } = rest as typeof rest & {
    onClick?: React.MouseEventHandler<HTMLButtonElement>;
  };
  const toast = useToast();
  const [state, setState] = React.useState<"idle" | "armed" | "firing">("idle");
  const timer = React.useRef<number | undefined>(undefined);

  React.useEffect(() => () => window.clearTimeout(timer.current), []);

  const onClick = (e: React.MouseEvent<HTMLButtonElement>) => {
    injectedClick?.(e);
    window.clearTimeout(timer.current);
    if (state === "idle") {
      setState("armed");
      if (warn) toast.warning(warn, { description: warnDescription });
      timer.current = window.setTimeout(() => setState("idle"), timeoutMs);
    } else if (state === "armed") {
      setState("firing");
      onConfirm();
      // Revert if the control is still mounted (e.g. the delete failed).
      timer.current = window.setTimeout(() => setState("idle"), 3000);
    }
  };

  return (
    <Button
      type="button"
      size={size}
      variant={state === "firing" ? "destructive" : state === "armed" ? "outline" : variant}
      disabled={disabled || state === "firing"}
      onClick={onClick}
      className={cn(
        state === "idle" &&
          variant === "ghost" &&
          "text-destructive hover:text-destructive",
        className,
        // Applied last so the armed cue wins over any caller className.
        state === "armed" &&
          "border-warning text-warning hover:bg-warning-soft",
      )}
      {...buttonProps}
    >
      {children}
    </Button>
  );
}
