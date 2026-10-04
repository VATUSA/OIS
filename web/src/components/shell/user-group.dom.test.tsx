// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it, vi} from "vitest";

/**
 * The User group's order (#680): What's new sits directly above Sign out, the one account action next
 * to the other, rather than among the navigation links. A later reorder would otherwise go unnoticed.
 */
vi.mock("@tanstack/react-router", () => ({
  Link: React.forwardRef<HTMLAnchorElement, {to: string; children?: React.ReactNode}>(({to, children, ...rest}, ref) => (
    <a ref={ref} href={to} {...rest}>
      {children}
    </a>
  )),
  useRouter: () => ({}),
}));
vi.mock("@/lib/auth", () => ({
  useMe: () => ({data: {id: "u1", display_name: "Test", cid: 1, permissions: ["api_keys.key.create"]}}),
  useLogout: () => ({mutate: () => undefined}),
}));
vi.mock("@/lib/permissions", () => ({hasPermission: () => true}));

import {UserGroup} from "./app-sidebar";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

let root: ReturnType<typeof createRoot> | undefined;
afterEach(() => {
  act(() => root?.unmount());
  root = undefined;
});

function rows(): {label: string; el: Element}[] {
  const host = document.createElement("div");
  root = createRoot(host);
  act(() =>
    root!.render(
      <ToastProvider>
        <UserGroup />
      </ToastProvider>,
    ),
  );
  return [...host.querySelectorAll("a, button")].map((el) => ({label: (el.textContent ?? "").trim(), el}));
}

describe("the sidebar's User group (#680)", () => {
  it("puts What's new directly above Sign out, after the navigation links", () => {
    const labels = rows().map((r) => r.label);
    expect(labels.slice(-3)).toEqual(["Desktop app", "What's new", "Sign out"]);
  });

  it("left-aligns What's new like the links around it", () => {
    const whatsNew = rows().find((r) => r.label === "What's new")!.el;
    expect(whatsNew.tagName).toBe("BUTTON");
    expect(whatsNew.className).toContain("text-left");
  });

  it("gives its icon the gold hover twinkle DESIGN.md names as the exception", () => {
    const icon = rows().find((r) => r.label === "What's new")!.el.querySelector("svg")!;
    const classes = icon.getAttribute("class") ?? "";
    expect(classes).toContain("group-hover/item:text-warning");
    expect(classes).toContain("motion-safe:group-hover/item:animate-sparkle");
    // The row's own hover colour is merged away, so the gold wins by construction rather than by
    // where Tailwind happens to emit the two rules.
    expect(classes).not.toContain("group-hover/item:text-ink-2");
  });
});
