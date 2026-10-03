import {existsSync, readdirSync, readFileSync, statSync} from "node:fs";
import {join} from "node:path";
import {describe, expect, it} from "vitest";

/**
 * Access presets stay deleted (VATUSA/OIS#550 AC1).
 *
 * Presets were a client-side constant that expanded into concrete permission rows. Because membership
 * was never recorded, "is this preset applied?" had to be reverse-engineered from the expanded set,
 * which is what #264 and #275 were. Groups replaced them; a preset coming back would bring the bug
 * class back with it.
 *
 * A source scan rather than a component test, because the regression is a *re-added file or import* —
 * something no test of an existing component can see.
 *
 * Scoped to the **access** presets on purpose. `pages/runway/index.tsx` (runway heading presets) and
 * `lib/hotkeys.ts` (hotkey combinations) use the word in unrelated senses and are not what this
 * forbids; a test matching the bare word would fail on both.
 */
function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((entry) => {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) return sourceFiles(path);
    return /\.tsx?$/.test(entry) && !/\.test\.tsx?$/.test(entry) ? [path] : [];
  });
}

/** An import of either deleted module, however the path is written. */
const OFFENDER = /from\s+["'](?:@\/lib\/presets|[./]+(?:lib\/)?presets|@\/components\/access\/preset-bar|[./]+(?:access\/)?preset-bar)["']/;

/** Comments describe the rule; only code can break it. */
function withoutComments(source: string): string {
  return source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "");
}

const SRC = new URL("..", import.meta.url).pathname;

describe("access presets stay removed (#550)", () => {
  it("leaves neither preset module in the tree", () => {
    expect(existsSync(join(SRC, "lib/presets.ts")), "web/src/lib/presets.ts").toBe(false);
    expect(existsSync(join(SRC, "components/access/preset-bar.tsx")), "preset-bar.tsx").toBe(false);
  });

  it("imports neither from anywhere", () => {
    const offenders = sourceFiles(SRC)
      .filter((path) => OFFENDER.test(withoutComments(readFileSync(path, "utf8"))))
      .map((path) => path.replace(/.*\/web\/src\//, "web/src/"));

    expect(offenders).toEqual([]);
  });

  it("would notice one — the pattern matches every way the import could be written", () => {
    // Without this, a regex that silently stopped matching would leave the test above passing
    // forever, which is the failure mode of every source scan.
    for (const line of [
      'import {ACCESS_PRESETS} from "@/lib/presets";',
      "import {BASE_PERMISSIONS} from '../lib/presets';",
      'import {ACCESS_PRESETS} from "./presets";',
      'import {PresetBar} from "@/components/access/preset-bar";',
      'import {PresetBar} from "./preset-bar";',
    ]) {
      expect(OFFENDER.test(line), line).toBe(true);
    }
    // ...and not the unrelated presets this scan is deliberately scoped away from.
    expect(OFFENDER.test('import {PRESETS} from "./runway-presets";')).toBe(false);
    expect(OFFENDER.test('import {useHotkeys} from "@/lib/hotkeys";')).toBe(false);
    // A mention in prose is not an import — this file's own doc comment is one.
    const prose = ["/**", ' * import {ACCESS_PRESETS} from "@/lib/presets"', " */"].join("\n");
    expect(OFFENDER.test(withoutComments(prose))).toBe(false);
  });
});
