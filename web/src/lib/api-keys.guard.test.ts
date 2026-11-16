import {mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync} from "node:fs";
import {tmpdir} from "node:os";
import {join, relative, sep} from "node:path";
import {fileURLToPath} from "node:url";
import {afterAll, beforeAll, describe, expect, it} from "vitest";

/**
 * The personal API-key token is shown once and never persisted client-side (#955).
 *
 * `POST /api/v1/api-keys` and `POST /api/v1/api-keys/{id}/rotate` return the plaintext token once.
 * It must live only in the reveal's React state, which dies with the page: a copy in web storage or a
 * script-set cookie outlives the reveal and is readable by any later XSS.
 *
 * This is a source scan because the property is an absence. No render of the page can see a
 * `localStorage.setItem` beside the reveal, and on #531 a reviewer added one with every behavioral
 * test green. Keep it a scan of every file the token passes through: `lib/api-keys.ts`, whose
 * mutations return it, and `pages/api-keys.tsx`, which holds and shows it.
 *
 * The scan flags any mention of a client-storage API, reads included: there is nothing legitimate for
 * these files to read back. `useLocalStorage` from `@ois/ui` is listed because it writes to
 * `localStorage` without the name appearing at the call site.
 *
 * Known limits: the comment pass is lexical, so a `//` in JSX text (`<a>https://x</a>`) blanks the
 * rest of that line, and a storage call later on the same line would be missed. And the file list is
 * checked against the files that name `ApiKeyToken`, so a file handed `token.token` as a plain string
 * (a reveal component with a `secret: string` prop) is not found and must be added to `FILES` by hand.
 */
const SRC = fileURLToPath(new URL("..", import.meta.url));
const FILES = ["lib/api-keys.ts", "pages/api-keys.tsx"];

/** Every non-test source file under `web/src`, relative to it, with `/` separators. */
function sources(dir: string): string[] {
  return readdirSync(dir, {withFileTypes: true}).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return sources(path);
    return /\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name)
      ? [relative(SRC, path).split(sep).join("/")]
      : [];
  });
}

/**
 * Comments blanked out, newlines kept, so a match's line number is still the real one.
 *
 * One left-to-right pass that matches string and template literals first and keeps them, so a `/*` or
 * `//` inside a string never starts a comment. Same pass as `api-paths.guard.test.ts`.
 */
const LITERAL_OR_COMMENT =
  /("(?:[^"\\\n]|\\.)*"|'(?:[^'\\\n]|\\.)*'|`(?:[^`\\]|\\.)*`)|\/\/[^\n]*|\/\*[\s\S]*?\*\//g;
function stripComments(text: string): string {
  return text.replace(LITERAL_OR_COMMENT, (m, literal?: string) =>
    literal ? m : m.replace(/[^\n]/g, " "),
  );
}

const STORAGE =
  /\b(?:localStorage|sessionStorage|indexedDB|openDatabase|cookieStore|caches|useLocalStorage)\b|\bdocument\s*\.\s*cookie\b|\bnavigator\s*\.\s*storage\b/g;

/** `line:api` for every client-storage access in the file. A file that cannot be read throws. */
function storageAccess(path: string): string[] {
  const code = stripComments(readFileSync(path, "utf8"));
  return [...code.matchAll(STORAGE)].map(
    ({0: api, index}) => `${code.slice(0, index).split("\n").length}:${api.replace(/\s/g, "")}`,
  );
}

describe("the personal API-key token never reaches client storage (#955)", () => {
  it.each(FILES)("%s contains no client-storage access", (file) => {
    expect(storageAccess(join(SRC, file)), `${file} must not persist the shown-once token`).toEqual([]);
  });

  // A scan of the wrong file, or of none, finds nothing and passes. The list must be exactly the
  // source files that name `ApiKeyToken`, so a dropped entry, or a new file that names it, fails here.
  it("scans exactly the source files that name ApiKeyToken", () => {
    const carriers = sources(SRC).filter((file) =>
      readFileSync(join(SRC, file), "utf8").includes("ApiKeyToken"),
    );
    expect(carriers.sort()).toEqual(FILES);
  });

  it("fails on a file it cannot read instead of scanning it empty", () => {
    expect(() => storageAccess(join(SRC, "lib/api-keys-missing.ts"))).toThrow(/ENOENT/);
  });
});

describe("the storage scan flags a write planted in a throwaway file", () => {
  let dir = "";
  let count = 0;
  /** Writes `src` to a new file outside the repo and scans it with the function the guard uses. */
  const scanPlanted = (src: string) => {
    const path = join(dir, `planted-${count++}.tsx`);
    writeFileSync(path, src);
    return storageAccess(path);
  };
  beforeAll(() => {
    dir = mkdtempSync(join(tmpdir(), "ois-955-guard-"));
  });
  afterAll(() => rmSync(dir, {recursive: true, force: true}));

  it.each([
    ['localStorage.setItem("ois.token", token.token);', "1:localStorage"],
    ["window.sessionStorage.setItem(key, token.token);", "1:sessionStorage"],
    ['const req = indexedDB.open("ois");', "1:indexedDB"],
    ['openDatabase("ois", "1", "", 1024);', "1:openDatabase"],
    ["document.cookie = `t=${token.token}`;", "1:document.cookie"],
    ["document . cookie = token.token;", "1:document.cookie"],
    ['await cookieStore.set("t", token.token);', "1:cookieStore"],
    ['await (await caches.open("ois")).put("/t", new Response(token.token));', "1:caches"],
    ["const root = await navigator.storage.getDirectory();", "1:navigator.storage"],
    ['const [, keep] = useLocalStorage("k", token.token);', "1:useLocalStorage"],
    // The key is a variable, not a literal.
    ["localStorage.setItem(key, token.token);", "1:localStorage"],
    // Bracket access, so the dotted name never appears.
    ['window["localStorage"].setItem(key, token.token);', "1:localStorage"],
  ])("flags %s", (src, expected) => {
    expect(scanPlanted(src)).toEqual([expected]);
  });

  // Prose that names a storage API is not a call. The live call on line 6 is the positive control:
  // a scan that matches nothing cannot pass this, and its line number proves newlines survive.
  it("ignores storage named only in comments and still flags the call after them", () => {
    const src = [
      "// keep the token out of localStorage",
      "/* sessionStorage and document.cookie are no better */",
      "/**",
      " * never useLocalStorage or cookieStore for it",
      " */",
      "localStorage.setItem(key, token.token);",
    ].join("\n");
    expect(scanPlanted(src)).toEqual(["6:localStorage"]);
  });

  // Read as comment starts, these would blank the rest of the file, the call included.
  it.each([
    ['const glob = "../assets/*.svg";\nlocalStorage.setItem(key, token.token);\n/** doc */', "2:localStorage"],
    ['const url = "https://example.org/a"; sessionStorage.setItem(key, token.token);', "1:sessionStorage"],
    ["const glob = '../assets/*.svg';\nlocalStorage.setItem(key, token.token);\n/** doc */", "2:localStorage"],
    ["const u = `https://ois/${id}`; localStorage.setItem(key, token.token);", "1:localStorage"],
  ])("still scans code after a comment opener inside a string: %s", (src, expected) => {
    expect(scanPlanted(src)).toEqual([expected]);
  });
});
