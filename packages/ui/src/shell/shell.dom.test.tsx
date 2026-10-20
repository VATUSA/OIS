// @vitest-environment jsdom
import * as React from "react";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, describe, expect, it} from "vitest";

import {CalendarClock, CalendarRange, Home, Sparkles, Wind} from "lucide-react";

import {TooltipProvider} from "../components/tooltip";
import {Shell, ShellContent, Sidebar, SidebarGroup, SidebarItem} from "./shell";

/**
 * The shell's own chrome (#402).
 *
 * Nothing asserted it before, and the shell wraps every signed-in page. The gutter, the outer rounded
 * corners and the frame shadow were removed so the app meets the window edges squarely. The *inner*
 * content panel keeps its rounding and margins — the part that is easy to remove by accident while
 * "flattening the shell", and now the only thing holding content off the window edge.
 */
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

function render(node: React.ReactNode): HTMLElement {
  const host = document.createElement("div");
  root = createRoot(host);
  act(() => root!.render(node));
  return host;
}

/** The shell's outermost element, and the frame inside it. */
function shellParts(host: HTMLElement) {
  const outer = host.firstElementChild as HTMLElement;
  return {outer, frame: outer.firstElementChild as HTMLElement};
}

describe("Shell chrome", () => {
  it("meets the viewport edges: no gutter, no outer radius, no shadow", () => {
    const {outer, frame} = shellParts(render(<Shell>page</Shell>));

    // A gutter of any breakpoint would put --ground back around the app.
    for (const gutter of ["p-2", "sm:p-3", "p-3"]) {
      expect(outer.className).not.toContain(gutter);
    }
    expect(frame.className).not.toContain("rounded");
    expect(frame.className).not.toContain("shadow");
    // Still full height, and still the panel surface the sidebar and main area sit on.
    expect(outer.className).toContain("h-dvh");
    expect(frame.className).toContain("bg-panel");
  });

  it("keeps the inner content panel inset and rounded", () => {
    const host = render(<Shell>page</Shell>);
    const panel = host.querySelector(".rounded-lg") as HTMLElement | null;

    expect(panel, "the inner content panel should survive flattening the frame").not.toBeNull();
    expect(panel!.className).toContain("border");
    // The inset is load-bearing now, not decoration: with the frame's gutter gone this margin is the
    // only thing holding the content panel off the window edge, so assert it rather than just the
    // rounding it is named after.
    expect(panel!.className).toContain("mx-2");
    expect(panel!.className).toContain("mb-2");
    expect(panel!.textContent).toBe("page");
  });

  it("does not paint a ground behind the frame, which would never be visible", () => {
    // The frame is the outer element's only child and stretches over the whole h-dvh box, so a
    // `bg-ground` out here is dead weight that reads as though a gutter were coming back.
    const {outer} = shellParts(render(<Shell>page</Shell>));

    expect(outer.className).not.toContain("bg-ground");
  });

  it("gets its side gutters from the content padding, not from the frame", () => {
    // AC2: with the frame's gutter gone, 16px at phone width has to come from here or pages sit
    // flush against the window edge.
    const host = render(
      <Shell>
        <ShellContent>body</ShellContent>
      </Shell>,
    );

    const padded = host.querySelector(".px-4") as HTMLElement | null;
    expect(padded, "ShellContent should still carry its own horizontal padding").not.toBeNull();
    expect(padded!.className).toContain("sm:px-6");
  });
});

describe("SidebarItem (#680)", () => {
  // A <button> centres its text by default, unlike the <a> every other item is; the item must not
  // depend on which element it is handed.
  it("left-aligns and fills the row whatever element it renders", () => {
    const host = render(
      <SidebarItem asChild icon={Sparkles} label="What's new">
        <button type="button" />
      </SidebarItem>,
    );
    const button = host.querySelector("button")!;
    expect(button.className).toContain("text-left");
    expect(button.className).toContain("w-full");
  });

  it("passes iconClassName to the icon", () => {
    const host = render(<SidebarItem href="#" icon={Sparkles} label="x" iconClassName="custom-icon" />);
    expect(host.querySelector("svg")!.getAttribute("class")).toContain("custom-icon");
  });
});

// Coverage matrix (VATUSA/OIS#823): the collapsed sidebar's group flyout, at the shell level
//
// Requirement                                   | Test                                                                         | Status
// ----------------------------------------------|------------------------------------------------------------------------------|-------
// AC1 one icon per labelled group, collapsed    | renders one trigger per labelled group with an icon …                        | done
// AC1 Home (label-less) stays a direct item     | renders a label-less group's items directly …                                | done
// AC1 fallback: labelled group without an icon  | renders today's markup for a labelled group with no icon …                   | done
// AC2 click opens to the right                  | opens the flyout on a primary pointer press …                                | done
// AC2 Enter / Space open                        | opens the flyout on Enter … / … on Space …                                   | done
// AC2 hover does not open                       | does not open the flyout on hover …                                          | done
// AC2 header + items with icon and label        | heads the flyout with the group name … / lists each item with its icon …     | done
// AC2 Escape closes, focus back to icon         | closes on Escape and returns focus to the group's trigger                    | done
// AC2 click outside closes                      | closes on a pointer press outside the flyout                                 | done
// AC2 navigation closes                         | closes an open flyout when the Sidebar's path changes …                      | done
// AC2 selecting an item closes                  | closes the flyout when an item is selected                                   | done
// Operator: Sign out keeps flyout open          | keeps the flyout open when a keep-open child is selected …                   | done
// AC3 aria-haspopup / aria-expanded             | marks the trigger aria-haspopup and aria-expanded false, then true …         | done
// AC3 aria-controls only while open             | sets aria-controls to the flyout's id while open and drops it when closed    | done
// AC3 aria-current in the flyout                | keeps an item's aria-current="page" inside the flyout                        | done
// AC4 active trigger                            | styles the trigger as the active nav tile … / leaves an inactive trigger …   | done
// AC6 expanded mode unchanged                   | renders identical markup expanded for a group with and without an icon …     | done
// AC7 drawer (no Sidebar) unchanged             | renders identical markup outside a Sidebar for a group with and without …    | done
//
// Radix opens a DropdownMenu on `pointerdown` (button 0, no ctrlKey), which jsdom does not synthesise
// from `.click()`; dispatch `new MouseEvent("pointerdown", {bubbles: true, cancelable: true, button: 0})`
// and stub `hasPointerCapture`/`setPointerCapture`/`releasePointerCapture`/`scrollIntoView` on
// `Element.prototype`, as `web/src/features/dashboard/add-widget-menu.dom.test.tsx` does. The content
// portals to `document.body`: query `document`, not the host, and clear `document.body` in `afterEach`.
describe("collapsed SidebarGroup flyout (#823)", () => {
  beforeAll(() => {
    const proto = Element.prototype as unknown as Record<string, unknown>;
    proto.hasPointerCapture ??= () => false;
    proto.setPointerCapture ??= () => {};
    proto.releasePointerCapture ??= () => {};
    proto.scrollIntoView ??= () => {};
  });

  let flyRoot: ReturnType<typeof createRoot> | undefined;
  afterEach(() => {
    act(() => flyRoot?.unmount());
    flyRoot = undefined;
    document.body.innerHTML = "";
  });

  function mount(node: React.ReactNode): HTMLElement {
    const host = document.createElement("div");
    document.body.appendChild(host);
    flyRoot = createRoot(host);
    act(() => flyRoot!.render(node));
    return host;
  }
  const rerender = (node: React.ReactNode) => act(() => flyRoot!.render(node));

  type RailProps = { collapsed?: boolean; path?: string; icon?: boolean; active?: boolean; inSidebar?: boolean };
  function Rail({ collapsed = true, path = "/a", icon = true, active, inSidebar = true }: RailProps) {
    const groups = (
      <>
        <SidebarGroup>
          <SidebarItem href="/" icon={Home} label="Home" />
        </SidebarGroup>
        <SidebarGroup label="Planning" icon={icon ? CalendarRange : undefined} active={active}>
          <SidebarItem href="/a" icon={CalendarClock} label="Events" aria-current="page" />
          <SidebarItem href="/b" icon={Wind} label="Configs" />
          <button type="button" data-keep-open>
            Arm
          </button>
        </SidebarGroup>
      </>
    );
    return <TooltipProvider>{inSidebar ? <Sidebar collapsed={collapsed} path={path}>{groups}</Sidebar> : groups}</TooltipProvider>;
  }

  const trigger = () => document.querySelector<HTMLButtonElement>("aside button[aria-haspopup]")!;
  const menu = () => document.querySelector<HTMLElement>('[role="menu"]');
  const press = (el: Element) =>
    act(() => {
      el.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, cancelable: true, button: 0 }));
    });
  const key = (el: Element, k: string) =>
    act(() => {
      el.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true }));
    });
  /** Radix registers its outside-press listener a tick after opening. */
  const tick = () => act(async () => new Promise((r) => setTimeout(r, 0)));

  it("renders one trigger per labelled group with an icon in a collapsed Sidebar, labelled by the group name, with no item links in the rail", () => {
    mount(<Rail />);
    const triggers = document.querySelectorAll("aside button[aria-haspopup]");
    expect(triggers).toHaveLength(1);
    expect(triggers[0].textContent).toBe("Planning");
    expect(triggers[0].querySelector(".sr-only")!.textContent).toBe("Planning");
    expect(document.querySelector('aside a[href="/a"]')).toBeNull();
    expect(document.querySelector('aside a[href="/b"]')).toBeNull();
  });

  it("renders a label-less group's items directly in a collapsed Sidebar, with no flyout trigger", () => {
    mount(<Rail />);
    const home = document.querySelector('aside a[href="/"]')!;
    expect(home).not.toBeNull();
    expect(home.closest("[aria-haspopup]")).toBeNull();
  });

  it("renders today's markup for a labelled group with no icon in a collapsed Sidebar, with no trigger", () => {
    mount(<Rail icon={false} />);
    expect(document.querySelector("aside [aria-haspopup]")).toBeNull();
    expect(document.querySelector('aside a[href="/a"]')).not.toBeNull();
    expect(document.querySelector('aside a[href="/b"]')).not.toBeNull();
  });

  it("opens the flyout on a primary pointer press on the trigger, placed to the right and aligned to the start", () => {
    mount(<Rail />);
    expect(menu()).toBeNull();
    press(trigger());
    const m = menu()!;
    expect(m).not.toBeNull();
    expect(m.closest("aside")).toBeNull();
    expect(m.getAttribute("data-side")).toBe("right");
    expect(m.getAttribute("data-align")).toBe("start");
  });

  it("opens the flyout on Enter on the focused trigger", () => {
    mount(<Rail />);
    act(() => trigger().focus());
    key(trigger(), "Enter");
    expect(menu()).not.toBeNull();
  });

  it("opens the flyout on Space on the focused trigger", () => {
    mount(<Rail />);
    act(() => trigger().focus());
    key(trigger(), " ");
    expect(menu()).not.toBeNull();
  });

  it("does not open the flyout on hover (pointerenter / mouseenter / mouseover), then opens it on a press", () => {
    mount(<Rail />);
    for (const type of ["pointerenter", "pointerover", "pointermove", "mouseenter", "mouseover"]) {
      act(() => {
        trigger().dispatchEvent(new MouseEvent(type, { bubbles: true }));
      });
    }
    expect(menu()).toBeNull();
    press(trigger());
    expect(menu()).not.toBeNull();
  });

  it("heads the flyout with the group name", () => {
    mount(<Rail />);
    press(trigger());
    expect(menu()!.firstElementChild!.textContent).toBe("Planning");
  });

  it("lists each item with its icon and a visible label, in the order given, as menu items", () => {
    mount(<Rail />);
    press(trigger());
    const items = [...menu()!.querySelectorAll('[role="menuitem"]')];
    expect(items.map((i) => i.textContent)).toEqual(["Events", "Configs", "Arm"]);
    for (const a of items.slice(0, 2)) {
      expect(a.tagName).toBe("A");
      expect(a.querySelector("svg")).not.toBeNull();
      // Visible: the collapsed sr-only variant needs a `data-collapsed` ancestor, and there is none.
      expect(a.closest("[data-collapsed]")).toBeNull();
    }
  });

  it("closes on Escape and returns focus to the group's trigger", async () => {
    mount(<Rail />);
    press(trigger());
    expect(menu()).not.toBeNull();
    key(document.activeElement ?? document.body, "Escape");
    await tick();
    expect(menu()).toBeNull();
    expect(document.activeElement).toBe(trigger());
  });

  it("closes on a pointer press outside the flyout", async () => {
    mount(<Rail />);
    press(trigger());
    await tick();
    expect(menu()).not.toBeNull();
    press(document.body);
    expect(menu()).toBeNull();
  });

  it("closes an open flyout when the Sidebar's path changes, and stays closed for a re-render at the same path", () => {
    mount(<Rail path="/a" />);
    press(trigger());
    rerender(<Rail path="/a" />);
    expect(menu()).not.toBeNull();
    rerender(<Rail path="/b" />);
    expect(menu()).toBeNull();
    rerender(<Rail path="/a" />);
    expect(menu()).toBeNull();
  });

  it("closes the flyout when an item is selected", async () => {
    mount(<Rail />);
    press(trigger());
    const configs = menu()!.querySelector<HTMLAnchorElement>('a[href="/b"]')!;
    // Stop jsdom following the link, after React has seen the click (a listener on the link itself
    // would run first, and Radix skips selection for a click already default-prevented).
    document.addEventListener("click", (e) => e.preventDefault(), { once: true });
    act(() => configs.click());
    await tick();
    expect(menu()).toBeNull();
  });

  it("keeps the flyout open when a keep-open child is selected, while a normal sibling item still closes it", async () => {
    mount(<Rail />);
    press(trigger());
    act(() => menu()!.querySelector<HTMLButtonElement>("button[data-keep-open]")!.click());
    await tick();
    expect(menu()).not.toBeNull();
    const configs = menu()!.querySelector<HTMLAnchorElement>('a[href="/b"]')!;
    // Stop jsdom following the link, after React has seen the click (a listener on the link itself
    // would run first, and Radix skips selection for a click already default-prevented).
    document.addEventListener("click", (e) => e.preventDefault(), { once: true });
    act(() => configs.click());
    await tick();
    expect(menu()).toBeNull();
  });

  it("marks the trigger aria-haspopup and aria-expanded false, then true once open", () => {
    mount(<Rail />);
    expect(trigger().getAttribute("aria-haspopup")).toBe("menu");
    expect(trigger().getAttribute("aria-expanded")).toBe("false");
    press(trigger());
    expect(trigger().getAttribute("aria-expanded")).toBe("true");
  });

  it("sets aria-controls to the flyout's id while open and drops it when closed", async () => {
    mount(<Rail />);
    expect(trigger().hasAttribute("aria-controls")).toBe(false);
    press(trigger());
    const id = trigger().getAttribute("aria-controls")!;
    expect(id).toBeTruthy();
    expect(document.getElementById(id)).toBe(menu());
    key(document.activeElement ?? document.body, "Escape");
    await tick();
    expect(trigger().hasAttribute("aria-controls")).toBe(false);
  });

  it('keeps an item\'s aria-current="page" inside the flyout and on no other item', () => {
    mount(<Rail />);
    press(trigger());
    const current = document.querySelectorAll('[aria-current="page"]');
    expect(current).toHaveLength(1);
    expect(current[0].getAttribute("href")).toBe("/a");
    expect(menu()!.contains(current[0])).toBe(true);
  });

  it("styles the trigger as the active nav tile with a text-brand-ink icon when the group is active", () => {
    mount(<Rail active />);
    const t = trigger();
    expect(t.hasAttribute("data-active")).toBe(true);
    expect(t.className).toContain("data-[active]:bg-panel-2");
    expect(t.querySelector("svg")!.getAttribute("class")).toContain("group-data-[active]/item:text-brand-ink");
  });

  it("leaves an inactive trigger without data-active or the active tile styling", () => {
    mount(<Rail active={false} />);
    expect(trigger().hasAttribute("data-active")).toBe(false);
  });

  it("renders identical markup expanded for a group with and without an icon and active", () => {
    const withIcon = mount(<Rail collapsed={false} active />).innerHTML;
    act(() => flyRoot!.unmount());
    document.body.innerHTML = "";
    const without = mount(<Rail collapsed={false} icon={false} />).innerHTML;
    expect(withIcon).toBe(without);
    expect(withIcon).not.toContain("aria-haspopup");
    expect(withIcon).toContain('href="/b"');
  });

  it("renders identical markup outside a Sidebar for a group with and without an icon, with no trigger", () => {
    const withIcon = mount(<Rail inSidebar={false} active />).innerHTML;
    act(() => flyRoot!.unmount());
    document.body.innerHTML = "";
    const without = mount(<Rail inSidebar={false} icon={false} />).innerHTML;
    expect(withIcon).toBe(without);
    expect(withIcon).not.toContain("aria-haspopup");
    expect(withIcon).toContain('href="/b"');
  });
});
