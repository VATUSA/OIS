import {readdir, readFile} from "node:fs/promises";
import {resolve} from "node:path";

import {beforeAll, describe, expect, it} from "vitest";

// Coverage matrix (VATUSA/OIS#1005)
//
// Requirement                                        | Test                                                        | Status
// ---------------------------------------------------|-------------------------------------------------------------|-------
// AC2 no shortcut hint uses a ⌘ literal              | no web or @ois/ui source outside shortcut-label.ts has ⌘    | done
// AC2 the helper is the one source of the label      | shortcut-label.ts is the only file with the ⌘ literal       | done
// guard is live (scan finds files)                   | file count above a floor; app-sidebar.tsx is in the set     | done
// guard is live (matcher matches)                    | a planted '"⌘K"' string in a code sample is flagged         | done
// guard catches escaped forms                        | \u2318, \u{2318}, &#8984;, &#x2318;, &#X2318; each flagged | done
// guard does not flag prose                          | ⌘K inside // and /* */ comments is not flagged              | done
//
// A source scan on purpose ("Absence needs a guard, not a grep", test-writing.md): a render test
// covers the sidebar only, and a new hint could be added anywhere. Mirror the ROOTS / strip / scan /
// sources shape of web/src/components/shell/window-chrome.guard.test.ts. Test files are excluded
// (their names and comments say "⌘K"). An unreadable file counts as a failure.

const ROOTS = [resolve(process.cwd(), "src"), resolve(process.cwd(), "../packages/ui/src")];
const HELPER = resolve(process.cwd(), "src/lib/shortcut-label.ts");
const SIDEBAR = resolve(process.cwd(), "src/components/shell/app-sidebar.tsx");

/** Comments out, so prose that mentions ⌘K is not a hint. Same limits as window-chrome.guard. */
function strip(text: string): string {
  return text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:])\/\/.*$/gm, "$1");
}

/** ⌘ as written, or as a JS escape (`\u2318`, `\u{2318}`) or an HTML entity (`&#8984;`, `&#x2318;`). */
const COMMAND_KEY = /⌘|\\u2318|\\u\{2318\}|&#8984;|&#x2318;/gi;

/** `file:line` for every ⌘ left in the code of `text`. */
function scan(file: string, text: string): string[] {
  const code = strip(text);
  return [...code.matchAll(COMMAND_KEY)].map((m) => `${file}:${code.slice(0, m.index).split("\n").length}`);
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

let files: string[] = [];
const hits = new Map<string, string[]>();
beforeAll(async () => {
  files = [];
  hits.clear();
  for (const root of ROOTS) files.push(...(await sources(root)));
  for (const file of files) {
    // An unreadable file is a failure, not a pass: readFile rejects and beforeAll fails the suite.
    hits.set(file, scan(file, await readFile(file, "utf8")));
  }
});

describe("shortcut hints come from shortcutLabel (VATUSA/OIS#1005 AC2)", () => {
  it("finds no ⌘ in the code of any web/src or packages/ui/src source other than web/src/lib/shortcut-label.ts", () => {
    const offenders = [...hits].filter(([file]) => file !== HELPER).flatMap(([, h]) => h);
    expect(offenders).toEqual([]);
  });
  it("finds the ⌘ literal in web/src/lib/shortcut-label.ts, so the helper is the one place it lives", () => {
    expect(hits.get(HELPER)?.length).toBeGreaterThan(0);
  });
  it("scans more than a floor of files and includes web/src/components/shell/app-sidebar.tsx", () => {
    expect(files.length).toBeGreaterThan(100);
    expect(files).toContain(SIDEBAR);
  });
  it("flags a planted <kbd>⌘K</kbd> and a planted label=\"Search (⌘K)\" in a code sample", () => {
    expect(scan("planted.tsx", 'const a = <kbd>⌘K</kbd>;\nconst b = <Button label="Search (⌘K)" />;\n')).toEqual([
      "planted.tsx:1",
      "planted.tsx:2",
    ]);
  });
  it("flags ⌘ written as a JS escape or an HTML entity, each on its own line", () => {
    const planted = ['"\\u2318K"', '"\\u{2318}K"', "<kbd>&#8984;K</kbd>", "<kbd>&#x2318;K</kbd>", "<kbd>&#X2318;K</kbd>"];
    for (const sample of planted) {
      expect(scan("planted.tsx", `const a = ${sample};\n`), sample).toEqual(["planted.tsx:1"]);
    }
  });
  it("does not flag ⌘K written in a // line comment or a /* block */ comment", () => {
    expect(scan("prose.tsx", "// open with ⌘K\n/* ⌘K\n   search */\nconst x = 1;\n")).toEqual([]);
  });
});
