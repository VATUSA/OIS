// @vitest-environment jsdom
import {describe, expect, it} from "vitest";

import {clampToMonitors, toLogical, type Monitor, type Rect} from "./popout-geometry";

const LAPTOP: Monitor = {position: {x: 0, y: 0}, size: {width: 1512, height: 982}, scaleFactor: 1};
const SECOND: Monitor = {position: {x: 1512, y: 0}, size: {width: 1920, height: 1080}, scaleFactor: 1};

const onSecond: Rect = {x: 2000, y: 200, width: 360, height: 500};

describe("clampToMonitors", () => {
  it("leaves a window where the user put it while that screen is still attached", () => {
    expect(clampToMonitors(onSecond, [LAPTOP, SECOND])).toEqual(onSecond);
  });

  it("rescues a window saved on a monitor that has since been unplugged", () => {
    // The whole point: otherwise it reopens at x=2000 on a 1512-wide laptop — nowhere.
    const rescued = clampToMonitors(onSecond, [LAPTOP])!;

    expect(rescued.x).toBeGreaterThanOrEqual(0);
    expect(rescued.x + rescued.width).toBeLessThanOrEqual(LAPTOP.size.width);
    expect(rescued.y).toBeGreaterThanOrEqual(0);
    expect(rescued.y + rescued.height).toBeLessThanOrEqual(LAPTOP.size.height);
  });

  it("keeps the size the user chose when it still fits", () => {
    const rescued = clampToMonitors(onSecond, [LAPTOP])!;
    expect(rescued.width).toBe(360);
    expect(rescued.height).toBe(500);
  });

  it("shrinks a window too large for the screen it is rescued onto", () => {
    const huge: Rect = {x: 5000, y: 5000, width: 4000, height: 3000};
    const rescued = clampToMonitors(huge, [LAPTOP])!;

    expect(rescued.width).toBe(LAPTOP.size.width);
    expect(rescued.height).toBe(LAPTOP.size.height);
  });

  it("keeps a window that is only partly off-screen, rather than jumping it about", () => {
    // Hanging off the right edge but mostly visible — moving it would be more annoying than helpful.
    const mostlyOn: Rect = {x: 1400, y: 100, width: 360, height: 500};
    expect(clampToMonitors(mostlyOn, [LAPTOP])).toEqual(mostlyOn);
  });

  it("rescues one that is technically touching but effectively invisible", () => {
    const sliver: Rect = {x: 1510, y: 100, width: 360, height: 500};
    expect(clampToMonitors(sliver, [LAPTOP])).not.toEqual(sliver);
  });

  it("defers to the OS when there is nothing saved or no monitors to check against", () => {
    expect(clampToMonitors(undefined, [LAPTOP])).toBeUndefined();
    expect(clampToMonitors(onSecond, [])).toBeUndefined();
  });
});

// Tauri reports window and monitor geometry in physical pixels, but a WebviewWindow is created
// from logical ones. Storing physical and restoring it as logical doubled a pop-out on every
// reopen on a 2x (Retina) display, compounding each cycle (VATUSA/OIS#349 review).
describe("geometry on a 2x display", () => {
  const RETINA: Monitor = {position: {x: 0, y: 0}, size: {width: 3024, height: 1964}, scaleFactor: 2};

  it("stores what Tauri reports as the logical rect the window was opened with", () => {
    const opened: Rect = {x: 100, y: 200, width: 420, height: 640};
    const reported = {x: 200, y: 400, width: 840, height: 1280}; // outerPosition/outerSize at 2x
    expect(toLogical(reported, 2)).toEqual(opened);
  });

  it("round-trips: a saved window reopens at the same size and place, not doubled", () => {
    const opened: Rect = {x: 100, y: 200, width: 420, height: 640};
    const saved = toLogical({x: 200, y: 400, width: 840, height: 1280}, 2);
    expect(clampToMonitors(saved, [RETINA])).toEqual(opened);
  });

  it("compares against the monitor in logical pixels, so a window near the far edge still counts", () => {
    // Logical width of this screen is 1512; a rect at logical x=1300 is on it. Compared in
    // physical pixels it would be too, but one at logical 1600 is off it — and must be re-homed.
    expect(clampToMonitors({x: 1300, y: 100, width: 200, height: 300}, [RETINA])).toEqual({
      x: 1300,
      y: 100,
      width: 200,
      height: 300,
    });
    expect(clampToMonitors({x: 1600, y: 100, width: 200, height: 300}, [RETINA])).toEqual({
      x: Math.round((1512 - 200) / 2),
      y: Math.round((982 - 300) / 2),
      width: 200,
      height: 300,
    });
  });
});
