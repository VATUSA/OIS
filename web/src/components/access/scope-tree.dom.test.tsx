// @vitest-environment jsdom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeAll, describe, expect, it} from "vitest";

import {PermissionScopeTree, type ScopeSelection} from "./scope-tree";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}

beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
});

const roots: {root: ReturnType<typeof createRoot>; host: HTMLElement}[] = [];
afterEach(() => {
  for (const {root, host} of roots.splice(0)) {
    act(() => root.unmount());
    host.remove();
  }
});

/**
 * The access tab and API-key picker still get their scope control after the shell was shared with the
 * Groups page (VATUSA/OIS#681): a checked item shows National / ARTCC chips, a facility-scoped one says so.
 */
describe("PermissionScopeTree on the shared shell", () => {
  it("renders the scope chips under a checked item and the facility-scoped note", async () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    roots.push({root, host});
    const selection: ScopeSelection = new Map([["tmu.adv.create", {national: true, artccs: []}]]);
    await act(async () => {
      root.render(
        <PermissionScopeTree
          items={[
            {name: "tmu.adv.create", bounds: {national: true, artccs: []}},
            {name: "tmu.adv.publish", bounds: {national: false, artccs: ["ZDC"]}},
          ]}
          facilities={[{id: "ZDC", name: "Washington"}]}
          selection={selection}
          onChange={() => {}}
        />,
      );
    });
    const tmu = [...host.querySelectorAll("button")].find((b) => b.textContent?.startsWith("tmu"))!;
    expect(tmu.textContent).toBe("tmu1");
    await act(async () => tmu.click());

    const buttons = [...host.querySelectorAll("button")].map((b) => b.textContent);
    expect(buttons).toContain("National");
    expect(buttons).toContain("ZDC");
    expect(host.textContent).toContain("(facility-scoped)");
  });
});
