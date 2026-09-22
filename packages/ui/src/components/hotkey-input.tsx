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

/** Whether we are on a Mac, where the Command key is named `Command` rather than `Super`. */
function onMac(): boolean {
  if (typeof navigator === "undefined") return false;
  const id = navigator.userAgent ?? "";
  return /Mac|iPhone|iPad/.test(id);
}

/**
 * Modifier order is fixed so the same combination always produces the same string.
 *
 * Command and Control are named **separately**, not collapsed into `CommandOrControl`. They are
 * different physical keys: on a Mac, folding Ctrl into `CommandOrControl` means the plugin
 * registers Command instead — so the combination the user pressed does nothing and one they never
 * chose is taken from every other application. The same applies to the Windows key.
 *
 * Returns `null` for anything that is not a usable shortcut, including a bare key with no modifier:
 * bound globally, `O` would fire every time the user typed the letter O anywhere.
 */
function acceleratorFrom(event: KeyboardEvent): string | null {
  const parts: string[] = [];
  if (event.metaKey) parts.push(onMac() ? "Command" : "Super");
  if (event.ctrlKey) parts.push("Control");
  if (event.altKey) parts.push("Alt");
  if (event.shiftKey) parts.push("Shift");

  const key = keyName(event);
  if (!key) return null;
  // No modifier means this would fire while the user types — refuse it at the source rather than
  // storing a binding that can only ever be rejected at registration.
  if (!parts.length) return null;

  parts.push(key);
  return parts.join("+");
}

/**
 * Which field is recording, app-wide.
 *
 * The settings page renders one of these per action. Without a single owner, clicking a second
 * field while a first is still armed leaves *both* listening, and one keypress is written into both
 * bindings — which then collide at registration and are reported as another application's fault.
 */
const captureSubscribers = new Set<(owner: object | null) => void>();
let captureOwner: object | null = null;

function claimCapture(owner: object) {
  captureOwner = owner;
  for (const notify of captureSubscribers) notify(captureOwner);
}

function releaseCapture(owner: object) {
  if (captureOwner !== owner) return;
  captureOwner = null;
  for (const notify of captureSubscribers) notify(null);
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
  onCaptureChange,
  placeholder = "Click, then press a shortcut",
  className,
  "aria-label": ariaLabel,
}: {
  value: string;
  onChange: (value: string) => void;
  /**
   * Called when this field starts and stops recording. The app uses it to hand the currently
   * registered shortcuts back to the OS while capturing — otherwise pressing the combination that
   * is already bound fires that shortcut instead of being recorded.
   */
  onCaptureChange?: (capturing: boolean) => void;
  placeholder?: string;
  className?: string;
  "aria-label"?: string;
}) {
  const [capturing, setCapturing] = React.useState(false);
  const rootRef = React.useRef<HTMLDivElement>(null);
  // Identity for the app-wide "who is capturing" token; never read, only compared.
  const identity = React.useRef({});

  // Only one field records at a time — see `captureSubscribers`.
  React.useEffect(() => {
    const notify = (owner: object | null) => {
      if (owner !== identity.current) setCapturing(false);
    };
    captureSubscribers.add(notify);
    return () => {
      captureSubscribers.delete(notify);
    };
  }, []);

  // Tell the app so it can suspend / resume the live registrations.
  const onCaptureChangeRef = React.useRef(onCaptureChange);
  onCaptureChangeRef.current = onCaptureChange;
  React.useEffect(() => {
    onCaptureChangeRef.current?.(capturing);
  }, [capturing]);

  // Give the token back when this field stops recording, so a later click can claim it.
  React.useEffect(() => {
    if (capturing) return;
    releaseCapture(identity.current);
  }, [capturing]);

  // Stop recording when the user's attention goes elsewhere.
  //
  // Without this, capture mode has no exit but Escape or finding this same button again — and
  // because it swallows every keydown in the document, the whole application's keyboard stays dead
  // in the meantime, with nothing on screen to explain it.
  React.useEffect(() => {
    if (!capturing) return;

    const onPointerDown = (event: Event) => {
      if (rootRef.current?.contains(event.target as Node)) return;
      setCapturing(false);
    };
    const onWindowBlur = () => setCapturing(false);

    document.addEventListener("pointerdown", onPointerDown, true);
    document.addEventListener("mousedown", onPointerDown, true);
    window.addEventListener("blur", onWindowBlur);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("mousedown", onPointerDown, true);
      window.removeEventListener("blur", onWindowBlur);
    };
  }, [capturing]);

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
    <div ref={rootRef} className="flex items-center gap-1.5">
      <button
        type="button"
        aria-label={ariaLabel}
        onClick={() =>
          setCapturing((on) => {
            if (on) return false;
            claimCapture(identity.current);
            return true;
          })
        }
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
