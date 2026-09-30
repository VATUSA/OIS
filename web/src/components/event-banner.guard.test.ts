import {readdirSync, readFileSync, statSync} from "node:fs";
import {join} from "node:path";
import {describe, expect, it} from "vitest";

/**
 * No page may point an `<img>` at an event's third-party banner URL (#429).
 *
 * The bundled desktop app runs under a CSP whose `img-src` allows `'self' data: blob:` and one map
 * CDN. Organisers host banners wherever they like — five unrelated hosts are already in the data —
 * so a raw `<img src={banner_image_url}>` is simply blocked there. It is invisible in development,
 * because `devCsp` is `null` and `just desktop` enforces no policy at all: the bug only appears in a
 * shipped build, which is exactly how it shipped.
 *
 * `EventBanner` is the only sanctioned way to render one. This scans the source rather than testing a
 * component, because the regression is a *new or reverted call site* — the thing a per-component test
 * cannot see.
 *
 * Limit, stated so nobody trusts it too far: it matches the field by name. A banner URL copied into
 * another variable first would slip through.
 */
function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((entry) => {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) return sourceFiles(path);
    return /\.tsx?$/.test(entry) && !/\.test\.tsx?$/.test(entry) ? [path] : [];
  });
}

const OFFENDER = /<img[^>]*\bsrc=\{[^}]*banner_image_url/;

/** Comments describe the rule; only code can break it. */
function withoutComments(source: string): string {
  return source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "");
}

describe("event banner call sites", () => {
  it("renders no banner through a direct <img src>", () => {
    const offenders = sourceFiles(new URL("..", import.meta.url).pathname)
      .filter((path) => OFFENDER.test(withoutComments(readFileSync(path, "utf8"))))
      .map((path) => path.replace(/.*\/web\/src\//, "web/src/"));

    expect(offenders).toEqual([]);
  });

  it("would notice one — the pattern really does match the shape it forbids", () => {
    // Without this, a regex that silently stopped matching would leave the test above passing
    // forever, which is the failure mode of every source scan.
    expect(OFFENDER.test('<img src={e.banner_image_url} alt="" />')).toBe(true);
    expect(OFFENDER.test("<EventBanner eventId={e.id} />")).toBe(false);
    // And a mention in prose is not a call site — this file's own doc comment is one.
    const prose = ["/**", " * a raw <img src={e.banner_image_url}> is blocked", " */"].join("\n");
    expect(OFFENDER.test(prose)).toBe(true);
    expect(OFFENDER.test(withoutComments(prose))).toBe(false);
  });
});
