import {readFileSync} from "node:fs";
import {describe, expect, it} from "vitest";

/**
 * The FCA pop-out follows `ladder.style` (VATUSA/OIS#557 AC2).
 *
 * It does so only because it renders the shared `Ladder` from `pages/fca/ladder.tsx`, which is where
 * the style is read. The DOM tests mount that `Ladder`, not the pop-out page, so a pop-out that drew
 * a ladder of its own would pass all of them while ignoring the setting — the divergence #349 was. A
 * source scan, because the regression is an edit to this file that no component test can see.
 */
const strip = (path: string) =>
  readFileSync(new URL(path, import.meta.url), "utf8")
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/^\s*\/\/.*$/gm, "")
    .replace(/\{\/\*[\s\S]*?\*\/\}/g, "");
const SOURCE = strip("./popout.tsx");
const ROUTER = strip("../router.tsx");
const AIRPORT = strip("./airport.tsx");

/** The body of one exported page function, up to the next top-level export. */
function page(name: string): string {
  const start = SOURCE.indexOf(`export function ${name}(`);
  expect(start, `${name} is defined`).toBeGreaterThanOrEqual(0);
  const next = SOURCE.indexOf("\nexport ", start + 1);
  return SOURCE.slice(start, next < 0 ? undefined : next);
}

describe("the FCA pop-out renders the shared ladder (#557 AC2)", () => {
  it("imports Ladder from the FCA ladder module and renders it", () => {
    expect(SOURCE).toMatch(/import\s*\{[^}]*\bLadder\b[^}]*\}\s*from\s*["']@\/pages\/fca\/ladder["']/);
    expect(SOURCE).toMatch(/<Ladder\b/);
  });

  it("draws no ladder of its own, which would bypass the style setting", () => {
    expect(SOURCE).not.toMatch(/<(ArrivalLadder|TguiLadder)\b/);
    expect(SOURCE).not.toMatch(/from\s*["'][^"']*components\/ladder\//);
  });
});

/**
 * The airport-ladder pop-out (VATUSA/OIS#790) follows `ladder.style` and draws the same gate columns
 * only because it renders the airport page's own `LadderView`, which is where both are decided. A
 * page that drew its own `TguiLadder` would pass every component test while ignoring the setting
 * and drifting from the page's columns. A source scan for the same reason as the FCA one above.
 */
describe("the airport pop-out renders the airport page's ladder (#790)", () => {
  it("is routed at /popout/airport/$icao", () => {
    expect(ROUTER).toMatch(/path:\s*["']popout\/airport\/\$icao["'][\s\S]*?component:\s*PopoutAirportLadderPage\b/);
  });

  it("imports LadderView from the airport page and renders it for the route's airport", () => {
    expect(SOURCE).toMatch(/import\s*\{[^}]*\bLadderView\b[^}]*\}\s*from\s*["']@\/pages\/airport["']/);
    const body = page("PopoutAirportLadderPage");
    expect(body).toMatch(/useParams\(\{\s*from:\s*["']\/popout\/airport\/\$icao["']\s*\}\)/);
    expect(body).toMatch(/useAirportFlow\(icao\)/);
    expect(body).toMatch(/<LadderView\b/);
    // Inside the pop-out, offering to pop out again would only raise the window it's already in.
    expect(body).not.toMatch(/popoutIcao/);
  });

  // The control's own behavior is pinned by `airport-popout.dom.test.tsx`; this pins that the page
  // actually asks for it, which no component test can see.
  it("is offered by the airport page's ladder tab", () => {
    expect(AIRPORT).toMatch(/<LadderView\b[^>]*\bpopoutIcao=\{icao\}/);
  });
});
