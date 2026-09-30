import {readFileSync} from "node:fs";
import {describe, expect, it} from "vitest";

/**
 * Pins the frameless window's controls onto every layout that renders outside `AppShell` (#423).
 *
 * This reads the source rather than rendering, because `router.tsx` imports every page in the app and
 * cannot be mounted in jsdom. A component test on `WindowChromeBar` proves the bar works; only this
 * proves it is still *wired up* — which is the regression that shipped: the controls were fine, they
 * just weren't on the signed-out landing page or the error screen.
 *
 * If a third non-shell layout is added, mount the bar in it and raise the count.
 */
const source = readFileSync(new URL("./router.tsx", import.meta.url), "utf8");

describe("router non-shell layouts", () => {
  it("mounts the window chrome bar in both layouts that render outside AppShell", () => {
    expect(source).toContain('from "@/components/shell/window-chrome-bar"');

    const mounts = source.match(/<WindowChromeBar\s*\/>/g) ?? [];
    // One for the backend-unreachable error screen, one for the signed-out landing page.
    expect(mounts).toHaveLength(2);
  });
});
