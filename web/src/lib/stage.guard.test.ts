import {readdir, readFile} from "node:fs/promises";
import {resolve} from "node:path";

import {describe, expect, it} from "vitest";

/**
 * The build configs read the repo-root STAGE file and fail closed (#841), and every desktop window
 * is created in one place that stages its title.
 *
 * A source scan, because vitest never executes `vite.config.ts`'s `define` for a chosen STAGE, and
 * the docs config has no test runner at all. Comments are stripped first, and each construct is
 * matched whole (the read, the parse and the use in one expression) so a fallback or a swapped
 * source cannot pass. The behavioral half, a bogus STAGE failing both builds, is run by hand.
 */
const WEB = process.cwd();
const ROOT = resolve(WEB, "..");

function strip(text: string): string {
  return text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:])\/\/.*$/gm, "$1");
}

async function code(file: string): Promise<string> {
  return strip(await readFile(resolve(ROOT, file), "utf8"));
}

/** The body of `function releaseStage() { … }`, asserted present before anything is said about it. */
function releaseStageBody(source: string): string {
  const match = /function releaseStage\(\)\s*\{([\s\S]*?)\n\}/.exec(source);
  expect(match, "function releaseStage() not found").not.toBeNull();
  return match![1]!;
}

async function sources(dir: string): Promise<string[]> {
  const out: string[] = [];
  for (const entry of await readdir(dir, {withFileTypes: true})) {
    const path = resolve(dir, entry.name);
    if (entry.isDirectory()) out.push(...(await sources(path)));
    else if (/\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name)) out.push(path);
  }
  return out;
}

describe("STAGE build wiring (#841)", () => {
  it("web/vite.config.ts defines __APP_STAGE__ from parseStage over the root STAGE file, with no fallback", async () => {
    const source = await code("web/vite.config.ts");
    expect(source).toMatch(/__APP_STAGE__:\s*JSON\.stringify\(releaseStage\(\)\)/);
    const body = releaseStageBody(source);
    expect(body).toMatch(/return parseStage\(readFileSync\(path\.resolve\(import\.meta\.dirname, "\.\.", "STAGE"\), "utf8"\)\);/);
    expect(body).not.toMatch(/\bcatch\b|\?\?|\|\|/);
  });

  it("docs-site/.vitepress/config.mts renders the nav version through navVersionText with the root STAGE file's stage", async () => {
    const source = await code("docs-site/.vitepress/config.mts");
    expect(source).toMatch(/\{\s*text:\s*navVersionText\(version\(\),\s*releaseStage\(\)\),\s*link:/);
    const body = releaseStageBody(source);
    expect(body).toMatch(/return parseStage\(readFileSync\(new URL\("\.\.\/\.\.\/STAGE", import\.meta\.url\), "utf8"\)\);/);
    expect(body).not.toMatch(/\bcatch\b|\?\?|\|\|/);
  });

  it('turbo.json lists "STAGE" in globalDependencies, so a STAGE edit is not served from cache', async () => {
    const turbo = JSON.parse(await readFile(resolve(ROOT, "turbo.json"), "utf8")) as {globalDependencies?: string[]};
    expect(turbo.globalDependencies).toContain("STAGE");
  });

  it("grants the main window core:window:allow-set-title, without which stageMainWindowTitle only warns", async () => {
    const capability = JSON.parse(await readFile(resolve(ROOT, "desktop/src-tauri/capabilities/default.json"), "utf8")) as {
      windows: string[];
      permissions: unknown[];
    };
    expect(capability.windows).toContain("main");
    expect(capability.permissions).toContain("core:window:allow-set-title");
  });

  it("constructs WebviewWindow only in web/src/lib/popout.ts, the one place window titles are staged", async () => {
    const files = await sources(resolve(WEB, "src"));
    // Positive control: a scan that found no files would pass with nothing scanned.
    expect(files.length).toBeGreaterThan(100);
    const constructing: string[] = [];
    for (const file of files) {
      if (/\bnew\s+WebviewWindow\s*\(/.test(strip(await readFile(file, "utf8")))) {
        constructing.push(file.slice(WEB.length + 1));
      }
    }
    // popout.dom.test.ts asserts the title that construction is given.
    expect(constructing).toEqual(["src/lib/popout.ts"]);
  });
});
