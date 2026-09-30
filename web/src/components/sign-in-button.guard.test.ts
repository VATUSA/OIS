import {readdir, readFile} from "node:fs/promises";
import {resolve} from "node:path";

import {beforeAll, describe, expect, it} from "vitest";

/**
 * Sign-in goes through `SignInButton`, and nothing else calls `login()` (#428).
 *
 * `sign-in-button.dom.test.tsx` proves the button invalidates `["me"]`, disables while in flight and
 * reports failures. It does not prove anyone *uses* it: reverting `landing.tsx` or `shared.tsx` to
 * `<Button onClick={login}>` — which is exactly bug #428 — left all 600 tests passing (#428 review).
 *
 * That is the whole history of this bug. `useLogin()` existed and was correct; two call sites had
 * drifted onto the bare `login()` and each showed the defect, because on desktop `login()` resolves
 * in place and only the `["me"]` invalidation can re-render the app signed in. So the guard is on the
 * call sites, and it catches a *fourth* one drifting as readily as one of these three reverting.
 */
const SRC = resolve(process.cwd(), "src");

/** `login` belongs to these two: the module that defines it, and the one component that calls it. */
const MAY_IMPORT_LOGIN = ["lib/auth.ts", "components/sign-in-button.tsx"];

async function sources(dir: string): Promise<string[]> {
  const out: string[] = [];
  for (const entry of await readdir(dir, {withFileTypes: true})) {
    const path = resolve(dir, entry.name);
    if (entry.isDirectory()) out.push(...(await sources(path)));
    else if (/\.tsx?$/.test(entry.name)) out.push(path);
  }
  return out;
}

/** A named import of `login` from the auth module — not `useLogin`, not `desktopLogin`. */
const IMPORTS_LOGIN = /import\s*\{([^}]*)\}\s*from\s*["'][^"']*\/auth["']/g;
const NAMES_LOGIN = (clause: string) =>
  clause.split(",").some((name) => name.trim().split(/\s+as\s+/)[0]?.trim() === "login");

let files: {path: string; text: string}[] = [];
beforeAll(async () => {
  files = await Promise.all(
    (await sources(SRC)).map(async (path) => ({
      path: path.slice(SRC.length + 1),
      text: await readFile(path, "utf8"),
    })),
  );
});

describe("sign-in call sites", () => {
  it("import login() nowhere but the auth module and SignInButton", () => {
    const offenders = files
      .filter((f) => !MAY_IMPORT_LOGIN.includes(f.path) && !f.path.includes(".test."))
      .filter((f) => [...f.text.matchAll(IMPORTS_LOGIN)].some((m) => NAMES_LOGIN(m[1]!)))
      .map((f) => f.path);

    expect(offenders).toEqual([]);
  });

  /**
   * The button is the only thing that offers sign-in, so any other control saying so has bypassed it
   * — which is what `<Button onClick={login}>Sign in with VATSIM</Button>` looked like on both pages.
   */
  it("offer sign-in only through SignInButton", () => {
    const offenders = files
      .filter((f) => f.path !== "components/sign-in-button.tsx" && !f.path.includes(".test."))
      .filter((f) => /Sign in with VATSIM/.test(f.text))
      .filter((f) => !/<SignInButton/.test(f.text))
      .map((f) => f.path);

    expect(offenders).toEqual([]);
  });

  /** All three known surfaces still render it, so removing one is not silently fine either. */
  it("render it on the landing page, the shared dashboard and the sidebar", () => {
    for (const path of [
      "pages/landing.tsx",
      "pages/dashboards/shared.tsx",
      "components/shell/app-sidebar.tsx",
    ]) {
      const file = files.find((f) => f.path === path);
      expect(file, `${path} should exist`).toBeDefined();
      expect(file!.text, `${path} should render <SignInButton`).toMatch(/<SignInButton/);
    }
  });

  /** The matchers have to actually see an offender, or the three above are decoration. */
  it("recognise a bare login() import and ignore useLogin", () => {
    expect([...`import {login, useMe} from "@/lib/auth";`.matchAll(IMPORTS_LOGIN)].some((m) => NAMES_LOGIN(m[1]!))).toBe(true);
    expect([...`import {useLogin} from "@/lib/auth";`.matchAll(IMPORTS_LOGIN)].some((m) => NAMES_LOGIN(m[1]!))).toBe(false);
    expect([...`import {desktopLogin} from "@/lib/desktop-auth";`.matchAll(IMPORTS_LOGIN)].some((m) => NAMES_LOGIN(m[1]!))).toBe(false);
  });
});
