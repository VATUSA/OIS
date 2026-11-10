import {readFileSync} from "node:fs";
import path from "node:path";

import {describe, expect, it} from "vitest";

import {BUILD_STAGE} from "./build-stage";
import {parseStage} from "./stage";

// Unmocked on purpose: the chip and popout tests mock `@/lib/build-stage`, so this is the only test
// that asserts the value the vite `define` bakes in. The expected side is read from the STAGE file,
// not pinned, so changing the stage needs no code edit (#841).
describe("BUILD_STAGE (#841)", () => {
  it("is the stage in the repo-root STAGE file, as the vite define injects it", () => {
    const raw = readFileSync(path.resolve(import.meta.dirname, "../../../STAGE"), "utf8");
    expect(BUILD_STAGE).toBe(parseStage(raw));
  });
});
