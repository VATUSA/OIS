// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, describe, expect, it, vi} from "vitest";

import {HotkeyInput} from "./hotkey-input";

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

function render(value: string, onChange: (v: string) => void) {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  act(() => root.render(<HotkeyInput value={value} onChange={onChange} aria-label="Focus OIS" />));
  return container.querySelector("button")!;
}

/** A press, as React reports it — `code` is the physical key, which is what the OS matches on. */
/**
 * A press, as the document sees it. `code` is the physical key, which is what the OS matches on.
 *
 * Dispatched on `document` rather than the button because WebKit doesn't focus a button on click,
 * so the component listens at the document level.
 */
function press(
  _button: HTMLElement,
  init: {code: string; key?: string} & Partial<KeyboardEventInit>,
) {
  act(() => {
    document.dispatchEvent(
      new KeyboardEvent("keydown", {bubbles: true, key: init.key ?? "a", ...init}),
    );
  });
}

/** jsdom reports a Linux-ish agent; the Command/Super naming depends on the platform. */
function pretendPlatform(userAgent: string) {
  Object.defineProperty(navigator, "userAgent", {value: userAgent, configurable: true});
}
const MAC = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)";
const WINDOWS = "Mozilla/5.0 (Windows NT 10.0; Win64; x64)";

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("HotkeyInput", () => {
  it("records the combination the user presses", () => {
    pretendPlatform(MAC);
    const onChange = vi.fn();
    const button = render("", onChange);

    act(() => button.click());
    press(button, {code: "KeyO", metaKey: true, shiftKey: true});

    expect(onChange).toHaveBeenCalledWith("Command+Shift+O");
  });

  it("keeps Command and Control apart", () => {
    // Collapsing both into `CommandOrControl` meant a Mac user pressing Ctrl got Command
    // registered instead: their combination did nothing, and one they never chose was taken from
    // every other application.
    pretendPlatform(MAC);
    const onChange = vi.fn();
    const button = render("", onChange);

    act(() => button.click());
    press(button, {code: "KeyO", ctrlKey: true, shiftKey: true});

    expect(onChange).toHaveBeenCalledWith("Control+Shift+O");
  });

  it("names the Windows key Super rather than Command", () => {
    pretendPlatform(WINDOWS);
    const onChange = vi.fn();
    const button = render("", onChange);

    act(() => button.click());
    press(button, {code: "KeyO", metaKey: true});

    expect(onChange).toHaveBeenCalledWith("Super+O");
  });

  it("ignores modifiers pressed on their own", () => {
    // Holding Command before reaching the letter must not record "CommandOrControl" as a shortcut.
    const onChange = vi.fn();
    const button = render("", onChange);

    act(() => button.click());
    press(button, {code: "MetaLeft", key: "Meta", metaKey: true});

    expect(onChange).not.toHaveBeenCalled();
  });

  it("uses the physical key, not the character it produces", () => {
    // ⌥O types "ø" and ⇧1 types "!" — neither is what the OS will match against.
    const onChange = vi.fn();
    const button = render("", onChange);

    act(() => button.click());
    press(button, {code: "Digit1", key: "!", ctrlKey: true, shiftKey: true});

    expect(onChange).toHaveBeenCalledWith("Control+Shift+1");
  });

  it("does not record a bare key with no modifier", () => {
    // Registered globally, that would fire whenever the user types that letter anywhere. It used
    // to be stored anyway and refused later at registration, which left a binding the settings
    // page displayed as if it worked.
    const onChange = vi.fn();
    const button = render("", onChange);

    act(() => button.click());
    press(button, {code: "KeyO"});

    expect(onChange).not.toHaveBeenCalled();
    // Still recording — the user just hasn't pressed a usable combination yet.
    expect(button.textContent).toContain("Press a shortcut");
  });

  it("clears the binding on Backspace", () => {
    const onChange = vi.fn();
    const button = render("CommandOrControl+Shift+O", onChange);

    act(() => button.click());
    press(button, {code: "Backspace", key: "Backspace"});

    expect(onChange).toHaveBeenCalledWith("");
  });

  it("records nothing when the user presses Escape", () => {
    const onChange = vi.fn();
    const button = render("CommandOrControl+Shift+O", onChange);

    act(() => button.click());
    press(button, {code: "Escape", key: "Escape"});

    expect(onChange).not.toHaveBeenCalled();
  });

  it("ignores keys pressed before the field is focused for capture", () => {
    const onChange = vi.fn();
    const button = render("", onChange);

    press(button, {code: "KeyO", metaKey: true, shiftKey: true});

    expect(onChange).not.toHaveBeenCalled();
  });

  it("stops recording when the user clicks somewhere else", () => {
    // Capture consumes every keydown in the document. With no way out but Escape or this same
    // button, clicking away left the whole application's keyboard dead and nothing on screen
    // saying why.
    const onChange = vi.fn();
    const button = render("", onChange);
    act(() => button.click());

    const elsewhere = document.createElement("input");
    document.body.appendChild(elsewhere);
    act(() => {
      elsewhere.dispatchEvent(new MouseEvent("mousedown", {bubbles: true}));
    });

    const event = new KeyboardEvent("keydown", {bubbles: true, cancelable: true, key: "x", code: "KeyX"});
    act(() => {
      document.dispatchEvent(event);
    });

    expect(event.defaultPrevented).toBe(false);
    expect(onChange).not.toHaveBeenCalled();
    expect(button.textContent).not.toContain("Press a shortcut");
    elsewhere.remove();
  });

  it("stops recording when the window loses focus", () => {
    const button = render("", vi.fn());
    act(() => button.click());

    act(() => {
      window.dispatchEvent(new Event("blur"));
    });

    expect(button.textContent).not.toContain("Press a shortcut");
  });

  it("shows the current binding when not capturing", () => {
    const button = render("CommandOrControl+Shift+O", vi.fn());
    expect(button.textContent).toContain("CommandOrControl+Shift+O");
  });
});

describe("only one field records at a time", () => {
  /** The settings page renders one of these per action, so two can be armed at once. */
  function renderTwo() {
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    const onA = vi.fn();
    const onB = vi.fn();
    act(() =>
      root.render(
        <>
          <HotkeyInput value="" onChange={onA} aria-label="A" />
          <HotkeyInput value="" onChange={onB} aria-label="B" />
        </>,
      ),
    );
    const [a, b] = Array.from(container.querySelectorAll("button"));
    return {a: a!, b: b!, onA, onB};
  }

  it("writes one press into one binding, not every armed field", () => {
    // Both fields listening meant one keypress was written to both settings — which then collided
    // at registration and was reported as another application's fault.
    const {a, b, onA, onB} = renderTwo();

    act(() => a.click());
    act(() => b.click());
    act(() => {
      document.dispatchEvent(
        new KeyboardEvent("keydown", {bubbles: true, code: "KeyT", key: "t", metaKey: true, shiftKey: true}),
      );
    });

    expect(onA).not.toHaveBeenCalled();
    expect(onB).toHaveBeenCalledTimes(1);
  });

  it("takes the recording indicator off the field that lost it", () => {
    const {a, b} = renderTwo();

    act(() => a.click());
    expect(a.textContent).toContain("Press a shortcut");

    act(() => b.click());
    expect(a.textContent).not.toContain("Press a shortcut");
    expect(b.textContent).toContain("Press a shortcut");
  });
});

describe("handing the shortcuts back while recording", () => {
  it("reports when capture starts and stops", () => {
    // A registered accelerator is swallowed by the OS, so the app has to release the bindings
    // while a new one is being recorded or an existing shortcut can never be moved.
    const onCaptureChange = vi.fn();
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    act(() =>
      root.render(
        <HotkeyInput value="" onChange={vi.fn()} onCaptureChange={onCaptureChange} aria-label="A" />,
      ),
    );
    const button = container.querySelector("button")!;

    onCaptureChange.mockClear();
    act(() => button.click());
    expect(onCaptureChange).toHaveBeenLastCalledWith(true);

    act(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", {bubbles: true, code: "Escape", key: "Escape"}));
    });
    expect(onCaptureChange).toHaveBeenLastCalledWith(false);
  });
});
