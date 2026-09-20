import * as React from "react";

import {cn} from "../lib/utils";

/**
 * Records a keyboard shortcut by *pressing* it, rather than asking the user to type its name.
 *
 * Pressing the combination is what people reach for, and expecting someone to know that
 * `CommandOrControl+Shift+O` is the literal text to type is a developer's mental model, not a
 * controller's.
 *
 * Emits the accelerator in the form Tauri's global-shortcut plugin parses.
 */

/** Modifier order is fixed so the same combination always produces the same string. */
function acceleratorFrom(event: KeyboardEvent): string | null {
  const parts: string[] = [];
  if (event.metaKey || event.ctrlKey) parts.push("CommandOrControl");
  if (event.altKey) parts.push("Alt");
  if (event.shiftKey) parts.push("Shift");

  const key = keyName(event);
  if (!key) return null;

  parts.push(key);
  return parts.join("+");
}

/**
 * The non-modifier key, named as the plugin expects.
 *
 * Uses `event.code` rather than `event.key`: with Shift or Option held, `key` is the *produced*
 * character (⌥O gives "ø", ⇧1 gives "!"), which is not the physical key the OS will match on.
 */
function keyName(event: KeyboardEvent): string | null {
  const code = event.code;
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  if (/^F[0-9]{1,2}$/.test(code)) return code;
  if (code === "Space") return "Space";
  if (code === "Enter") return "Enter";
  if (/^Arrow(Up|Down|Left|Right)$/.test(code)) return code.slice(5);
  // A modifier on its own is not a shortcut — keep waiting for a real key.
  return null;
}

export function HotkeyInput({
  value,
  onChange,
  placeholder = "Click, then press a shortcut",
  className,
  "aria-label": ariaLabel,
}: {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  className?: string;
  "aria-label"?: string;
}) {
  const [capturing, setCapturing] = React.useState(false);

  // Listen on the document rather than on the button.
  //
  // WebKit (which is what Tauri renders with on macOS) does not focus a <button> when it is
  // clicked, so an onKeyDown on the element never fires — the first version of this recorded
  // nothing at all. Capturing at the document level doesn't depend on focus behaviour.
  React.useEffect(() => {
    if (!capturing) return;

    const onKeyDown = (event: KeyboardEvent) => {
      // Consume everything while capturing so Tab, Escape and ⌘-combinations can't leak away.
      event.preventDefault();
      event.stopPropagation();

      if (event.key === "Escape") {
        setCapturing(false);
        return;
      }
      if (event.key === "Backspace" || event.key === "Delete") {
        onChange("");
        setCapturing(false);
        return;
      }

      const accelerator = acceleratorFrom(event);
      if (!accelerator) return; // Modifiers alone — wait for the key they modify.

      onChange(accelerator);
      setCapturing(false);
    };

    // Capture phase, so nothing else in the app reacts to the keystroke first.
    document.addEventListener("keydown", onKeyDown, true);
    return () => document.removeEventListener("keydown", onKeyDown, true);
  }, [capturing, onChange]);

  return (
    <div className="flex items-center gap-1.5">
      <button
        type="button"
        aria-label={ariaLabel}
        onClick={() => setCapturing((on) => !on)}
        className={cn(
          "h-9 min-w-56 rounded-md border px-3 text-left font-mono text-sm transition-colors",
          capturing
            ? "border-brand bg-panel-2 text-ink"
            : "border-line bg-panel-2 text-ink hover:bg-card",
          className,
        )}
      >
        {capturing ? "Press a shortcut…" : value || <span className="text-ink-3">{placeholder}</span>}
      </button>
      {value && !capturing && (
        <button
          type="button"
          onClick={() => onChange("")}
          className="text-xs text-ink-3 transition-colors hover:text-ink"
          aria-label="Clear shortcut"
        >
          Clear
        </button>
      )}
    </div>
  );
}
