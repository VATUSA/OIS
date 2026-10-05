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
const SOURCE = readFileSync(new URL("./popout.tsx", import.meta.url), "utf8")
  .replace(/\/\*[\s\S]*?\*\//g, "")
  .replace(/^\s*\/\/.*$/gm, "")
  .replace(/\{\/\*[\s\S]*?\*\/\}/g, "");

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
