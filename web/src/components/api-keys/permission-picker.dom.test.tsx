// @vitest-environment jsdom
//
// VATUSA/OIS#550 AC4, through the component. `mergeGroupIntoSelection` has its own tests, but they
// would stay green if the "Start from" control stopped calling it — so this drives the real select.
import {act} from "react";
import {createRoot} from "react-dom/client";
import {QueryClient, QueryClientProvider} from "@tanstack/react-query";
import {ToastProvider} from "@ois/ui";
import {afterEach, beforeAll, describe, expect, it} from "vitest";

import type {GrantablePermission} from "@/lib/api-keys";
import type {HeldGroup} from "@/lib/groups";

import {type PermSelection, PermissionPicker} from "./permission-picker";

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

const national = (permission: string): GrantablePermission => ({ permission, national: true, artccs: [] });
const TRAFFIC = ["tmu.programs.update", "flow.fca.update", "stats.history.read"];
const GRANTABLE = [...TRAFFIC, "events.config.update"].map(national);

/** Mount the picker with `groups` already in the cache — the query is seeded, never fetched. */
function mount(groups: HeldGroup[], selection: PermSelection = new Map()) {
  const changes: PermSelection[] = [];
  const qc = new QueryClient({defaultOptions: {queries: {retry: false, staleTime: Infinity}}});
  qc.setQueryData(["access-self", "groups"], groups);

  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  roots.push({root, host});
  act(() =>
    root.render(
      <QueryClientProvider client={qc}>
        <ToastProvider>
          <PermissionPicker
            grantable={GRANTABLE}
            facilities={[{id: "ZDC", name: "Washington"}]}
            selection={selection}
            onChange={(next) => changes.push(next)}
          />
        </ToastProvider>
      </QueryClientProvider>,
    ),
  );
  const select = host.querySelector<HTMLSelectElement>('select[aria-label="Start from one of your groups"]');
  return {host, select, changes};
}

function choose(select: HTMLSelectElement, value: string) {
  act(() => {
    select.value = value;
    select.dispatchEvent(new Event("change", {bubbles: true}));
  });
}

describe("PermissionPicker 'Start from a group' (VATUSA/OIS#550)", () => {
  it("offers the creator's own groups", () => {
    const {select} = mount([{name: "NTMO", permissions: TRAFFIC}]);

    expect(select).not.toBeNull();
    const options = [...select!.options].map((o) => o.value);
    expect(options).toContain("NTMO");
  });

  /** The wiring: choosing a group must actually merge its permissions into the key. */
  it("merges the chosen group's permissions into the selection", () => {
    const {select, changes} = mount([{name: "NTMO", permissions: TRAFFIC}]);

    choose(select!, "NTMO");

    expect(changes).toHaveLength(1);
    expect([...changes[0].keys()].sort()).toEqual([...TRAFFIC].sort());
  });

  /** #275 through the UI: an existing broader grant survives starting from a group. */
  it("keeps an existing grant exactly as it was (#275)", () => {
    const existing: PermSelection = new Map([["events.config.update", {national: true, artccs: []}]]);
    const {select, changes} = mount([{name: "NTMO", permissions: TRAFFIC}], existing);

    choose(select!, "NTMO");

    expect(changes[0].get("events.config.update")).toEqual({national: true, artccs: []});
    expect(changes[0].size).toBe(TRAFFIC.length + 1);
  });

  /**
   * #264 through the UI: there is no "applied" state. The control resets to its placeholder rather
   * than showing a group as selected, so nothing can ever claim a group is applied.
   */
  it("shows no group as applied after choosing one (#264)", () => {
    const {select} = mount([{name: "NTMO", permissions: TRAFFIC}]);

    choose(select!, "NTMO");

    expect(select!.value).toBe("");
  });

  /** A group that would add nothing — already selected, or nothing delegable — is not offered. */
  it("does not offer a group that would add nothing", () => {
    const allSelected: PermSelection = new Map(TRAFFIC.map((p) => [p, {national: true, artccs: []}]));
    const {select} = mount([{name: "NTMO", permissions: TRAFFIC}], allSelected);

    expect([...select!.options].map((o) => o.value)).not.toContain("NTMO");
    expect(select!.disabled).toBe(true);
  });

  /** "Remove all" was the one part of the preset bar that wasn't a preset, so it must survive. */
  it("still offers Remove all", () => {
    const {host} = mount([{name: "NTMO", permissions: TRAFFIC}]);

    expect(host.textContent).toContain("Remove all");
  });
});
