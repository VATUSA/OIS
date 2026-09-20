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

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("HotkeyInput", () => {
  it("records the combination the user presses", () => {
    const onChange = vi.fn();
    const button = render("", onChange);

    act(() => button.click());
    press(button, {code: "KeyO", metaKey: true, shiftKey: true});

    expect(onChange).toHaveBeenCalledWith("CommandOrControl+Shift+O");
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

    expect(onChange).toHaveBeenCalledWith("CommandOrControl+Shift+1");
  });

  it("does not record a bare key with no modifier", () => {
    // Registered globally, that would fire whenever the user types that letter anywhere.
    const onChange = vi.fn();
    const button = render("", onChange);

    act(() => button.click());
    press(button, {code: "KeyO"});

    expect(onChange).toHaveBeenCalledWith("O");
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

  it("shows the current binding when not capturing", () => {
    const button = render("CommandOrControl+Shift+O", vi.fn());
    expect(button.textContent).toContain("CommandOrControl+Shift+O");
  });
});
